//! A 한글 document's print: its pages drawn as the screen draws them, at 300 dots an inch, on
//! several threads at once, and put in a PDF for the printer (a picture a page).
//!
//! rhwp's own PDF writer lays every page's text out again with fonts it reads from their files
//! for each glyph run, half a second a page and no faster on more cores. Drawing the page is
//! what the screen does already, with the fonts read once ([`super::render::fonts_for`]): a print
//! of 80 pages is ready in seconds, and shows on paper what the tab shows.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use lopdf::{Document, Object, Stream, dictionary};
use resvg::usvg;

use super::render;

/// Dots an inch a page is printed at.
const DPI: f32 = 300.0;
/// The most threads a print is drawn on (each holds a page's pixels, 35 MB at 300 dpi).
const MOST_THREADS: usize = 6;
/// JPEG quality of the pages.
const QUALITY: u8 = 90;

/// A page drawn for print: its size (points) and its picture (JPEG, its pixels' size).
struct Printed {
    size: [f32; 2],
    pixels: [u32; 2],
    jpeg: Vec<u8>,
}

/// The print PDF of `pages` (their SVG), drawn on several threads; `done` counts the pages drawn.
pub(super) fn make(pages: &[String], done: &Arc<AtomicUsize>) -> Result<Vec<u8>, String> {
    if pages.is_empty() {
        return Err("no pages".into());
    }
    let threads = std::thread::available_parallelism().map_or(1, usize::from).clamp(1, MOST_THREADS).min(pages.len());
    let run = pages.len().div_ceil(threads);
    let runs: Vec<Result<Vec<Printed>, String>> = std::thread::scope(|scope| {
        let workers: Vec<_> = pages
            .chunks(run)
            .map(|chunk| {
                let done = done.clone();
                scope.spawn(move || {
                    let mut options = usvg::Options::default();
                    chunk
                        .iter()
                        .map(|svg| {
                            options.fontdb = render::fonts_for(svg);
                            let page = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| print_page(svg, &options)))
                                .unwrap_or_else(|_| Err("a page couldn't be drawn".into()));
                            done.fetch_add(1, Ordering::Relaxed);
                            page
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap_or_else(|_| Err("a page couldn't be drawn".into()))).collect()
    });
    let mut printed = Vec::with_capacity(pages.len());
    for run in runs {
        printed.extend(run?);
    }
    pdf(printed).map_err(|e| format!("the PDF couldn't be written: {e}"))
}

/// One page drawn for print and made a JPEG.
fn print_page(svg: &str, options: &usvg::Options) -> Result<Printed, String> {
    let scale = DPI / 72.0;
    let (size, rgba) = render::draw(svg, scale, options).ok_or("a page couldn't be drawn")?;
    let (w, h) = (u32::try_from(size[0]).map_err(|e| e.to_string())?, u32::try_from(size[1]).map_err(|e| e.to_string())?);
    // On white paper every pixel is opaque: its premultiplied colour is its colour.
    let rgb: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|[r, g, b, _]| [*r, *g, *b]).collect();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, QUALITY)
        .encode(&rgb, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    Ok(Printed { size: [w as f32 / scale, h as f32 / scale], pixels: [w, h], jpeg })
}

/// A PDF of the pages, each its picture filling it.
fn pdf(pages: Vec<Printed>) -> Result<Vec<u8>, lopdf::Error> {
    let mut doc = Document::with_version("1.5");
    let tree = doc.new_object_id();
    let mut kids = Vec::with_capacity(pages.len());
    for page in pages {
        let [w, h] = page.size;
        let picture = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => i64::from(page.pixels[0]),
                "Height" => i64::from(page.pixels[1]),
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
                "Filter" => "DCTDecode",
            },
            page.jpeg,
        );
        let picture = doc.add_object(picture);
        let content = doc.add_object(Stream::new(dictionary! {}, format!("q {w:.3} 0 0 {h:.3} 0 0 cm /P Do Q").into_bytes()));
        let id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => tree,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(w), Object::Real(h)],
            "Contents" => content,
            "Resources" => dictionary! { "XObject" => dictionary! { "P" => picture } },
        });
        kids.push(Object::Reference(id));
    }
    let count = kids.len() as i64;
    doc.objects.insert(tree, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => tree });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page of `w` × `h` CSS pixels with a black square in its corner.
    fn page(w: u32, h: u32) -> String {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}"><rect width="96" height="96" fill="black"/></svg>"#)
    }

    #[test]
    fn pages_are_drawn_on_several_threads_into_one_pdf() {
        let pages: Vec<String> = (0..7).map(|i| page(794, 1123 + i)).collect();
        let done = Arc::new(AtomicUsize::new(0));
        let made = make(&pages, &done).unwrap();
        assert_eq!(done.load(Ordering::Relaxed), 7);
        let doc = Document::load_mem(&made).unwrap();
        assert_eq!(doc.get_pages().len(), 7);
        // In page order, each its own size in points (CSS pixels × 0.75).
        let heights: Vec<f32> = doc
            .get_pages()
            .values()
            .map(|id| doc.get_dictionary(*id).unwrap().get(b"MediaBox").unwrap().as_array().unwrap()[3].as_float().unwrap())
            .collect();
        for (i, h) in heights.iter().enumerate() {
            assert!((h - (1123 + i) as f32 * 0.75).abs() < 0.5, "page {i}: {h}");
        }
        assert!(make(&[], &done).is_err());
        assert!(make(&["not svg".to_owned()], &done).is_err());
    }
}
