//! The deck: the presentation's slides in order, each drawn over its background with its master's
//! and layout's shapes behind its own, in one page that fits them to the window.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::color;
use crate::package::{self, Node, PX, Package, at, esc, flag, int, kid, kids, pt, rid};
use crate::scene::{Ctx, Out, Space};
use crate::shape::{Scene, Styles};
use crate::style::{self, Theme};
use crate::table::TableStyles;
use crate::text::{self, Levels};
use crate::{Deck, Labels, NOT_PPTX};

/// What every slide shares.
struct Shared {
    pkg: Package,
    default: Levels,
    tables: TableStyles,
    labels: Labels,
}

pub fn deck(bytes: &[u8], title: &str, labels: &Labels) -> Result<Deck, String> {
    let pkg = Package::read(bytes).ok_or(NOT_PPTX)?;
    let main = pkg.rels("").first("officeDocument").unwrap_or("ppt/presentation.xml").to_string();
    let xml = pkg.text(&main).ok_or(NOT_PPTX)?;
    let doc = package::parse(&xml).ok_or(NOT_PPTX)?;
    let root = doc.root_element();
    let size = kid(root, "sldSz").map_or((720.0, 540.0), |s| (int(s, "cx").map_or(720.0, pt), int(s, "cy").map_or(540.0, pt)));
    let rels = pkg.rels(&main);
    let slides: Vec<String> = kid(root, "sldIdLst")
        .map(|l| kids(l, "sldId").filter_map(|s| rid(s, "id").and_then(|id| rels.part(id)).map(str::to_string)).collect())
        .unwrap_or_default();
    let tables = TableStyles::parse(rels.first("tableStyles").and_then(|p| pkg.text(p)).as_deref());
    let shared = Shared { default: text::levels(kid(root, "defaultTextStyle")), tables, labels: labels.clone(), pkg };
    let mut out = Out::default();
    for (i, part) in slides.iter().enumerate() {
        slide(&shared, &mut out, part, i + 1);
    }
    let html = page(title, size, &out.html);
    Ok(Deck { html, media: std::mem::take(&mut out.files), slides: slides.len(), size })
}

/// A part and its XML.
struct Part {
    rels: package::Rels,
    xml: Option<String>,
}

impl Part {
    fn read(pkg: &Package, part: Option<&str>) -> Part {
        Part { rels: part.map(|p| pkg.rels(p)).unwrap_or_default(), xml: part.and_then(|p| pkg.text(p)) }
    }
}

/// One slide: its background, then the master's shapes, the layout's and its own.
fn slide(sh: &Shared, out: &mut Out, part: &str, number: usize) {
    let pkg = &sh.pkg;
    let own = Part::read(pkg, Some(part));
    let Some(doc) = own.xml.as_deref().and_then(package::parse) else { return };
    let root = doc.root_element();
    let layout = Part::read(pkg, own.rels.first("slideLayout"));
    let master = Part::read(pkg, layout.rels.first("slideMaster"));
    let theme = master.rels.first("theme").and_then(|p| pkg.text(p)).map(|x| Theme::parse(&x)).unwrap_or_default();
    let layout_doc = layout.xml.as_deref().and_then(package::parse);
    let master_doc = master.xml.as_deref().and_then(package::parse);
    let (layout_root, master_root) = (layout_doc.as_ref().map(|d| d.root_element()), master_doc.as_ref().map(|d| d.root_element()));
    let map = color_map(master_root, layout_root, root);
    let slide_ctx = Ctx { pkg, theme: &theme, map: &map, rels: own.rels };
    let layout_ctx = Ctx { pkg, theme: &theme, map: &map, rels: layout.rels };
    let master_ctx = Ctx { pkg, theme: &theme, map: &map, rels: master.rels };
    let styles = styles(&sh.default, master_root);
    let scene = Scene {
        layout: layout_root,
        master: master_root,
        layout_ctx: &layout_ctx,
        master_ctx: &master_ctx,
        styles: &styles,
        tables: &sh.tables,
        labels: &sh.labels,
        number,
    };
    let hidden = flag(root, "show") == Some(false);
    let background = background(out, [(Some(root), &slide_ctx), (layout_root, &layout_ctx), (master_root, &master_ctx)]);
    let note = if hidden { format!(" · {}", esc(&sh.labels.hidden)) } else { String::new() };
    let _ = write!(
        out.html,
        "<section class=\"s{}\" id=\"s{number}\"><div class=\"n\">{number}{note}</div><div class=\"slide\" style=\"background:{}\">",
        if hidden { " h" } else { "" },
        esc(&background)
    );
    let shows_master = flag(root, "showMasterSp") != Some(false);
    let layout_shows_master = layout_root.and_then(|l| flag(l, "showMasterSp")) != Some(false);
    if shows_master
        && layout_shows_master
        && let Some(t) = sp_tree(master_root)
    {
        scene.tree(out, t, &master_ctx, Space::SLIDE, true);
    }
    if shows_master && let Some(t) = sp_tree(layout_root) {
        scene.tree(out, t, &layout_ctx, Space::SLIDE, true);
    }
    if let Some(t) = sp_tree(Some(root)) {
        scene.tree(out, t, &slide_ctx, Space::SLIDE, false);
    }
    out.html.push_str("</div></section>");
}

/// A slide's, layout's or master's shapes.
fn sp_tree<'a, 'i>(root: Option<Node<'a, 'i>>) -> Option<Node<'a, 'i>> {
    root.and_then(|n| at(n, &["cSld", "spTree"]))
}

/// The text styles of a master: the presentation's under each of its title, body and other ones.
fn styles(default: &Levels, master: Option<Node>) -> Styles {
    let list = |name: &str| text::levels(master.and_then(|m| at(m, &["txStyles", name])));
    Styles { default: default.clone(), title: list("titleStyle"), body: list("bodyStyle"), other: list("otherStyle") }
}

/// The slide's colour map: the master's, then the layout's and the slide's overrides.
fn color_map(master: Option<Node>, layout: Option<Node>, slide: Node) -> HashMap<String, String> {
    let mut map: HashMap<String, String> =
        [("bg1", "lt1"), ("tx1", "dk1"), ("bg2", "lt2"), ("tx2", "dk2")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let mut take = |n: Option<Node>| {
        for a in n.into_iter().flat_map(|n| n.attributes()) {
            map.insert(a.name().to_string(), a.value().to_string());
        }
    };
    take(master.and_then(|m| kid(m, "clrMap")));
    take(layout.and_then(|l| at(l, &["clrMapOvr", "overrideClrMapping"])));
    take(at(slide, &["clrMapOvr", "overrideClrMapping"]));
    map
}

/// The slide's background as CSS: its own, else its layout's, else its master's; white without.
fn background(out: &mut Out, layers: [(Option<Node>, &Ctx); 3]) -> String {
    for (root, ctx) in layers {
        let Some(bg) = root.and_then(|r| at(r, &["cSld", "bg"])) else { continue };
        if let Some(f) = kid(bg, "bgPr").and_then(|pr| style::fill(pr, &ctx.rels)) {
            return ctx.paint(out, &f, None).css().unwrap_or_else(|| "#fff".into());
        }
        if let Some(r) = kid(bg, "bgRef") {
            let ph = color::within(r).map(|c| ctx.rgba(&c, None));
            if let Some(f) = ctx.theme.fill(int(r, "idx").unwrap_or(0)) {
                return ctx.paint(out, f, ph).css().unwrap_or_else(|| "#fff".into());
            }
        }
    }
    "#fff".into()
}

const CSS: &str = "html{background:#e9eaee}\
body{margin:0;font-family:'Malgun Gothic',sans-serif}\
.deck{padding:20px 24px 40px}\
.s{width:max-content;margin:0 auto 28px}\
.n{font-size:12px;color:#5f6368;margin:0 0 6px 2px}\
.slide{position:relative;overflow:hidden;background:#fff;box-shadow:0 1px 4px rgba(0,0,0,.28);zoom:var(--z,1)}\
.h .slide{opacity:.55}\
.sp{position:absolute;box-sizing:border-box}\
.sp img{position:absolute;max-width:none}\
.geo{position:absolute;left:0;top:0;width:100%;height:100%;overflow:visible}\
.tx{position:absolute;inset:0;display:flex;flex-direction:column;box-sizing:border-box;overflow-wrap:break-word;word-break:keep-all}\
.tf{display:flex;flex-direction:column;box-sizing:border-box;overflow-wrap:break-word;word-break:keep-all}\
.tx p,.tf p{margin:0;white-space:pre-wrap;tab-size:72pt}\
.nw p{white-space:pre}\
.t{position:absolute;border-collapse:collapse;table-layout:fixed}\
.t td{padding:0;box-sizing:border-box;overflow:hidden}\
.chart{position:absolute;display:flex;align-items:center;justify-content:center;box-sizing:border-box;border:1px dashed #9aa0a6;background:rgba(241,243,244,.85);color:#5f6368;font-size:14pt;text-align:center;padding:8pt}";

/// The page: the slides one under another, fitted to the window's width as it changes.
fn page(title: &str, (w, h): (f64, f64), slides: &str) -> String {
    let script = format!(
        "(function(){{var W={:.1};function fit(){{var z=Math.max(.15,Math.min(2.5,(document.documentElement.clientWidth-48)/W));document.documentElement.style.setProperty('--z',z);}}addEventListener('resize',fit);fit();}})();",
        w * PX
    );
    format!(
        "<!doctype html><html lang=\"ko\"><head><meta charset=\"utf-8\"><title>{}</title><style>{CSS}.slide{{width:{w:.2}pt;height:{h:.2}pt}}</style></head><body><main class=\"deck\">{slides}</main><script>{script}</script></body></html>",
        esc(title)
    )
}
