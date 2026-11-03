//! Document tabs: Markdown text in a tab of its own beside the worksheets (see `crate::docs`).

use serde_json::{Value as Json, json};

use super::{CommandSpec, EngineError, PROTECTED, Result, Session, bad, cmd, edit, has_doc, str_param, target_sheet};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "doc.insert",
            "New Document",
            ["Insert", "Text"],
            None,
            "{name?, prefix?: \"Doc\" (an unnamed document takes the first free <prefix>N), before?: index, text?}: a Markdown document tab after the active sheet",
            has_doc,
            insert
        ),
        cmd!("doc.setText", "Edit Document", [], None, "{sheet?, text}: replaces a document tab's Markdown text", has_doc, set_text),
        cmd!(query "doc.get", "Get Document", [], None, "{sheet?} → {name, text}", has_doc, get),
    ]
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
    Ok(json!({"name": sh.name, "text": doc.text}))
}
