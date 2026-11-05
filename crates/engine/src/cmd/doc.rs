//! Document tabs: Markdown text in a tab of its own beside the worksheets (see `crate::docs`).

use serde_json::{Value as Json, json};

use super::{CommandSpec, EngineError, PROTECTED, Result, Session, bad, cmd, documents_allowed, edit, has_doc, str_param, target_sheet};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "doc.insert",
            "New Document",
            ["Insert", "Text"],
            None,
            "{name?, prefix?: \"Doc\" (an unnamed document takes the first free <prefix>N), before?: index, text?}: a Markdown document tab after the active sheet",
            documents_allowed,
            insert
        ),
        cmd!("doc.setText", "Edit Document", [], None, "{sheet?, text}: replaces a document tab's Markdown text", has_doc, set_text),
        cmd!(query "doc.get", "Get Document", [], None, "{sheet?} → {name, text, mode, fitWidth} (mode: edit|split|view, null until one is chosen)", has_doc, get),
        cmd!(
            noundo "doc.view",
            "Document View",
            [],
            None,
            "{sheet?, mode?: edit|split|view, fitWidth?: bool} → {mode, fitWidth}: how a document tab shows (its text, both side by side, or the document) and its Fit Width. Kept in the file, so the workbook changes; no undo step",
            has_doc,
            view
        ),
        cmd!(
            "doc.import",
            "Import Markdown",
            ["Insert", "Text"],
            None,
            "{path, before?: index}: a Markdown file (.md, UTF-8) as a new document tab named after it",
            documents_allowed,
            import
        ),
        cmd!(noundo "doc.export", "Export as Markdown", [], None, "{path, sheet?}: writes a document tab's text to a Markdown file (UTF-8)", has_doc, export),
    ]
}

fn import(s: &mut Session, p: &Json) -> Result<Json> {
    let path = str_param(p, "path").ok_or_else(|| bad("path", "required"))?.to_owned();
    let text = crate::docs::markdown_text(&crate::io::read_file(&path)?);
    // Named after the file; a name in use gets " (2)", " (3)", … as a copy does.
    let base = crate::docs::name_from_path(&path).unwrap_or_else(|| "Doc".into());
    let wb = &s.doc()?.wb;
    let name = std::iter::once(base.clone())
        .chain((2..1000).map(|n| {
            let tail = format!(" ({n})");
            let head: String = base.chars().take(31 - tail.chars().count()).collect();
            format!("{head}{tail}")
        }))
        .find(|n| wb.sheet_index(n).is_none())
        .ok_or_else(|| bad("path", "no free tab name"))?;
    let mut params = json!({"name": name});
    if let Some(before) = p.get("before") {
        params["before"] = before.clone();
    }
    super::sheet::insert_tab(s, &params, Some(text))
}

fn export(s: &mut Session, p: &Json) -> Result<Json> {
    let path = str_param(p, "path").ok_or_else(|| bad("path", "required"))?.to_owned();
    let sheet = target_sheet(s, p)?;
    let sh = s.doc()?.wb.sheet(sheet).ok_or(EngineError::NoDocument)?;
    let doc = sh.document.as_ref().ok_or_else(|| EngineError::Other(crate::docs::NOT_A_DOCUMENT.into()))?;
    let bytes = doc.text.clone().into_bytes();
    crate::io::write_file(&path, &bytes)?;
    Ok(json!({"path": path, "bytes": bytes.len()}))
}

fn insert(s: &mut Session, p: &Json) -> Result<Json> {
    let text = crate::docs::normalize(str_param(p, "text").unwrap_or_default());
    super::sheet::insert_tab(s, p, Some(text))
}

fn set_text(s: &mut Session, p: &Json) -> Result<Json> {
    let sheet = target_sheet(s, p)?;
    let text = crate::docs::normalize(str_param(p, "text").ok_or_else(|| bad("text", "required"))?);
    let sh = s.doc()?.wb.sheet(sheet).ok_or(EngineError::NoDocument)?;
    let doc = sh.document.as_ref().ok_or_else(|| bad("sheet", format!("`{}` is not a document", sh.name)))?;
    if doc.text == text {
        return Ok(json!({"changed": false}));
    }
    if sh.protection.is_some() {
        return Err(EngineError::Other(PROTECTED.into()));
    }
    edit(s, |cx| {
        if let Some(doc) = cx.wb.sheet_mut(sheet).and_then(|sh| sh.document.as_mut()) {
            doc.text = text;
        }
        Ok(json!({"changed": true}))
    })
}

fn get(s: &mut Session, p: &Json) -> Result<Json> {
    let sheet = target_sheet(s, p)?;
    let sh = s.doc()?.wb.sheet(sheet).ok_or(EngineError::NoDocument)?;
    let doc = sh.document.as_ref().ok_or_else(|| bad("sheet", format!("`{}` is not a document", sh.name)))?;
    Ok(json!({"name": sh.name, "text": doc.text, "mode": doc.mode.map(monun_model::DocMode::name), "fitWidth": doc.fit_width}))
}

/// Sets how a document tab shows. Like the text, the choice is the workbook's, kept in its file
/// (an XLSX file in the tab's mark), but it is no edit to take back: switching modes leaves Undo
/// for the text.
fn view(s: &mut Session, p: &Json) -> Result<Json> {
    let sheet = target_sheet(s, p)?;
    let mode = match str_param(p, "mode") {
        Some(m) => Some(monun_model::DocMode::parse(m).ok_or_else(|| bad("doc.view", format!("`mode` must be edit, split or view, not `{m}`")))?),
        None => None,
    };
    let fit = super::bool_param(p, "fitWidth");
    let d = s.doc_mut()?;
    let sh = d.wb.sheet(sheet).ok_or(EngineError::NoDocument)?;
    let doc = sh.document.as_ref().ok_or_else(|| bad("sheet", format!("`{}` is not a document", sh.name)))?;
    let (mode, fit) = (mode.or(doc.mode), fit.unwrap_or(doc.fit_width));
    if (mode, fit) != (doc.mode, doc.fit_width) {
        // A Markdown file keeps its text only, so how it shows leaves it as saved.
        let saved = d.is_markdown() && !d.is_dirty();
        let wb = std::sync::Arc::make_mut(&mut d.wb);
        if let Some(doc) = wb.sheet_mut(sheet).and_then(|sh| sh.document.as_mut()) {
            doc.mode = mode;
            doc.fit_width = fit;
        }
        d.revision += 1;
        if saved {
            d.saved = d.wb.clone();
        }
    }
    Ok(json!({"mode": mode.map(monun_model::DocMode::name), "fitWidth": fit}))
}
