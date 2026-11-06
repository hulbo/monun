use super::{
    INTERFACE_SCALE_MAX, INTERFACE_SCALE_MIN, Json, NoticeKind, SheetApp, UiRequest, backstage, clamp_interface_scale, clean_error, dialogs,
    formula_locale, i18n, json, now_ms, printing,
};

impl SheetApp {
    /// Runs an engine command (or a UI command) and handles UI requests it makes.
    pub fn run(&mut self, id: &str, params: Json) -> Result<Json, String> {
        let started = now_ms();
        let r = self.run_command(id, params);
        self.perf.note_command(id, now_ms() - started);
        // This copy leaves the system clipboard alone: note what it holds, so Paste can tell
        // that Monun's copy is newer while that text is still there.
        if matches!(id, "edit.copy" | "edit.cut") && r.is_ok() {
            self.clipboard_text_at_copy = self.system_clipboard_text();
            self.cut_text = if id == "edit.cut" { self.clipboard_text_at_copy.clone() } else { None };
        }
        r
    }

    pub(super) fn run_command(&mut self, id: &str, params: Json) -> Result<Json, String> {
        let switches_window = id.starts_with("window.")
            || matches!(id, "view.newWindow" | "view.hideWindow" | "view.unhideWindow")
            || (id == "view.switchWindows" && ["window", "doc"].iter().any(|k| params.get(*k).is_some_and(|d| !d.is_null())));
        if id.starts_with("file.") || switches_window {
            self.settle_drafts()?;
        }
        if matches!(id, "file.save" | "file.saveAs" | "file.saveBytes") {
            self.store_view_state();
        }
        if matches!(
            id,
            "sheet.activate"
                | "sheet.next"
                | "sheet.previous"
                | "sheet.move"
                | "sheet.hide"
                | "sheet.unhide"
                | "home.insertSheet"
                | "home.deleteSheet"
                | "edit.undo"
                | "edit.redo"
                | "object.delete"
                | "object.move"
                | "sheet.rename"
                | "doc.insert"
                | "doc.setText"
        ) {
            self.commit_text_box_edit()?;
            self.commit_doc_draft()?;
        }
        if let Some(r) = self.run_ui_command(id, &params) {
            return r;
        }
        let before = self.session.active_window();
        // Where each Markdown file was being read is kept before it closes.
        if matches!(id, "file.close" | "window.close" | "file.closeAll") {
            self.remember_open_doc_views();
        }
        let r = self.session.run(id, params);
        // A Markdown file's view lives outside its .md: kept by path, and given back on open (not
        // to a file already open, which shows as it is).
        if let Ok(result) = &r {
            match id {
                "doc.view" | "view.zoom" | "view.zoom100" => self.remember_doc_view(),
                "file.open" if result.get("alreadyOpen").is_none() => self.recall_doc_view(),
                _ => {}
            }
        }
        if id.starts_with("file.") && self.session.active_window() != before {
            // File › New or Open from the Backstage view: the window it was in goes back to its sheet.
            self.backstage = None;
        }
        // Requests the command made (a dialog, a draft) belong to the window it left active.
        self.follow_active_window();
        self.after_engine();
        if id == "view.arrangeAll"
            && self.native_windows
            && let Ok(r) = &r
        {
            let arrange = r.get("arrange").and_then(Json::as_str).unwrap_or("tiled").to_string();
            let windows = r.get("windows").and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_u64).collect()).unwrap_or_default();
            self.arrange_windows = Some((arrange, windows));
        }
        // The new chart is selected, as after inserting any object, so Delete removes it, not its data.
        if id == "insert.chart"
            && let Ok(r) = &r
            && let Some(chart) = r.get("chart").and_then(Json::as_u64).and_then(|id| u32::try_from(id).ok())
        {
            self.selected_chart = Some(chart);
        }
        if id == "file.exportPdf"
            && let Ok(r) = &r
        {
            self.deliver_pdf(r);
        }
        r
    }

    /// Keeps how the active Markdown or text file shows (mode, Fit Width, zoom, the line it is
    /// read at), by its path.
    pub(crate) fn remember_doc_view(&mut self) {
        if let Some(uid) = self.session.active().map(|d| d.uid) {
            self.remember_view_of(uid);
        }
    }

    /// [`Self::remember_doc_view`] for every open Markdown or text file: before files close, and
    /// when 모눈 quits.
    pub fn remember_open_doc_views(&mut self) {
        let uids: Vec<u64> = self.session.documents().iter().filter(|d| d.is_text_file() && d.path.is_some()).map(|d| d.uid).collect();
        for uid in uids {
            self.remember_view_of(uid);
        }
    }

    fn remember_view_of(&mut self, uid: u64) {
        let Some(d) = self.session.documents().iter().find(|d| d.uid == uid && d.is_text_file()) else { return };
        let sheet = d.wb.active_sheet();
        let (Some(path), Some(sh)) = (d.path.clone(), d.wb.active()) else { return };
        let Some(doc) = sh.document.as_ref() else { return };
        let kept = self.ui.doc_views.iter().find(|v| v.path == path).map_or(0, |v| v.line);
        let line = crate::doc_view::top_line(self, (uid, sheet)).unwrap_or(kept);
        let mode = doc.mode.map(|m| m.name().to_string()).unwrap_or_default();
        let memory = crate::DocViewMemory { path, mode, fit_width: doc.fit_width, zoom: sh.zoom, line };
        self.ui.doc_views.retain(|v| v.path != memory.path);
        self.ui.doc_views.insert(0, memory);
        self.ui.doc_views.truncate(crate::DOC_VIEWS_MAX);
    }

    /// Shows a Markdown or text file just opened as it was last shown.
    fn recall_doc_view(&mut self) {
        let Some(path) = self.session.active().filter(|d| d.is_text_file()).and_then(|d| d.path.clone()) else { return };
        let Some(v) = self.ui.doc_views.iter().find(|v| v.path == path).cloned() else { return };
        let mut view = json!({"fitWidth": v.fit_width});
        if !v.mode.is_empty() {
            view["mode"] = json!(v.mode);
        }
        let _ = self.session.run("doc.view", view);
        if v.zoom != 100 && (10..=400).contains(&v.zoom) {
            let _ = self.session.run("view.zoom", json!({"percent": v.zoom}));
        }
        if let Some(d) = self.session.active() {
            let doc = (d.uid, d.wb.active_sheet());
            crate::doc_view::restore_line(self, doc, v.line);
        }
    }

    pub fn run_or_alert(&mut self, id: &str, params: Json) {
        // A menu/shortcut opens the settings first; the Print page's button submits explicitly.
        if id == "file.print" {
            if self.services.print_native.is_some() {
                self.open_backstage(backstage::Page::Print);
                return;
            }
            return self.print_now(params);
        }
        // Menu-style Export as PDF with no file named: ask where to save, like Save As does.
        // Programmatic callers use `run` and keep getting the PDF bytes back.
        if id == "file.exportPdf" && self.services.pick_save.is_some() && params.get("path").and_then(Json::as_str).is_none_or(str::is_empty) {
            return self.export_pdf_with_picker(params);
        }
        // Recommended Charts with no chart type named (ribbon button, keytip, command search) asks
        // which chart to insert; programmatic callers of `run` still get the first recommendation.
        if id == "insert.recommendedCharts" && params.get("type").is_none() {
            return self.open_dialog("recommendedCharts", json!({}));
        }
        // Switch Windows with no window named (key tip, command search) opens the View tab's
        // list to choose from; programmatic callers of `run` get the list back.
        if id == "view.switchWindows" && ["window", "doc"].iter().all(|k| params.get(*k).is_none_or(Json::is_null)) {
            self.ui.ribbon_tab = "View".into();
            self.ui.ribbon_collapsed = false;
            self.open_switch_windows = true;
            return;
        }
        match self.run(id, params) {
            // A refused cell draft already shows its own message (with the rule's title).
            Err(e) if self.message.as_ref().is_some_and(|(_, m)| *m == e) => {}
            Err(e) => self.alert(e),
            Ok(_) => {}
        }
    }

    /// Shows a command error: a toast for an unavailable command, else a message box.
    pub(crate) fn alert(&mut self, e: String) {
        // Shown in the window the user ends up in, also after a command that switched windows.
        self.follow_active_window();
        if e.contains("is not available right now") {
            self.toast = Some((e, now_ms()));
        } else {
            self.message = Some(("Monun".into(), clean_error(&e)));
        }
    }

    /// UI-only commands (dialogs, view toggles that aren't saved in the file).
    fn run_ui_command(&mut self, id: &str, p: &Json) -> Option<Result<Json, String>> {
        let r = match id {
            "app.checkForUpdates" | "help.checkForUpdates" => {
                self.check_for_updates();
                Ok(Json::Null)
            }
            "ui.printSettings" => printing::settings(&mut self.printing, p, self.services.printers.as_ref()),
            "ui.printSubmit" => self.submit_print(p.clone()),
            "ui.ribbonTab" => {
                self.ui.ribbon_tab = p.get("tab").and_then(Json::as_str).unwrap_or("Home").to_string();
                Ok(Json::Null)
            }
            "view.collapseRibbon" => {
                // A document tab shows no ribbon: Ctrl+F1 there leaves the sheets' ribbon alone.
                if !crate::doc_view::is_active(self) {
                    self.ui.ribbon_collapsed = !self.ui.ribbon_collapsed;
                }
                Ok(Json::Null)
            }
            "view.expandFormulaBar" => {
                self.ui.formula_bar_expanded = p.get("on").and_then(Json::as_bool).unwrap_or(!self.ui.formula_bar_expanded);
                Ok(json!({"on": self.ui.formula_bar_expanded}))
            }
            "view.darkMode" => {
                let current = if self.ui.system_theme { self.effective_dark } else { self.ui.dark };
                self.ui.dark = p.get("on").and_then(Json::as_bool).unwrap_or(!current);
                self.ui.system_theme = false;
                Ok(json!({"on": self.ui.dark}))
            }
            "view.theme" => {
                match p.get("mode").and_then(Json::as_str) {
                    Some("system") => self.ui.system_theme = true,
                    Some(mode @ ("light" | "dark")) => {
                        self.ui.dark = mode == "dark";
                        self.ui.system_theme = false;
                    }
                    _ => return Some(Err("theme mode must be system, light, or dark".into())),
                }
                Ok(json!({"mode": self.ui.theme_mode()}))
            }
            "view.accent" => {
                match p.get("color").and_then(Json::as_str).and_then(crate::theme::Accent::parse) {
                    Some(accent) => self.ui.accent = accent,
                    None => {
                        let names: Vec<&str> = crate::theme::Accent::ALL.iter().map(|a| a.name()).collect();
                        return Some(Err(format!("accent color must be one of {}", names.join(", "))));
                    }
                }
                Ok(json!({"color": self.ui.accent.name()}))
            }
            "view.animations" => {
                self.ui.animations = p.get("on").and_then(Json::as_bool).unwrap_or(!self.ui.animations);
                Ok(json!({"on": self.ui.animations}))
            }
            "options.autoComplete" => {
                self.ui.autocomplete = p.get("on").and_then(Json::as_bool).unwrap_or(!self.ui.autocomplete);
                Ok(json!({"on": self.ui.autocomplete}))
            }
            "view.interfaceScale" => {
                // `{"percent": 125}` or `{"percent": "auto"}`; out-of-range values are clamped.
                let percent = match p.get("percent") {
                    Some(Json::String(s)) if s.eq_ignore_ascii_case("auto") => Some(100),
                    Some(v) => v.as_f64().and_then(clamp_interface_scale),
                    None => None,
                };
                match percent {
                    Some(v) => {
                        self.ui.interface_scale = v;
                        Ok(json!({"percent": v}))
                    }
                    None => Err(format!("missing `percent` ({INTERFACE_SCALE_MIN}–{INTERFACE_SCALE_MAX}, or \"auto\")")),
                }
            }
            "app.language.set" => {
                // `{"language": "ja"}`; `code` is accepted as an alias.
                let code = p.get("language").or_else(|| p.get("code")).and_then(Json::as_str).unwrap_or("");
                match i18n::Language::parse(code) {
                    Some(l) => {
                        self.ui.language = l;
                        Ok(json!({"language": l}))
                    }
                    None => Err(format!("unknown language {code:?} (use one of {})", i18n::Language::code_list(", "))),
                }
            }
            "app.language.english" => {
                self.ui.language = i18n::Language::En;
                Ok(json!({"language": i18n::Language::En}))
            }
            "app.language.japanese" => {
                self.ui.language = i18n::Language::Ja;
                Ok(json!({"language": i18n::Language::Ja}))
            }
            "ui.backstage" => {
                // `{"page": "info"}` opens File › Info; `{"page": "close"}` returns to the sheet.
                match p.get("page").and_then(Json::as_str).map(str::trim) {
                    Some("close" | "none" | "") => self.backstage = None,
                    Some(id) => match backstage::Page::parse(id) {
                        Some(page) => self.open_backstage(page),
                        None => {
                            return Some(Err(format!(
                                "unknown File page {id:?} (use home, new, open, info, saveAs, print, export, options or close)"
                            )));
                        }
                    },
                    None => self.open_backstage(backstage::Page::Home),
                }
                Ok(json!({"page": self.backstage.as_ref().map(|b| b.page.id())}))
            }
            "ui.dialog" => {
                let name = p.get("name").and_then(Json::as_str).unwrap_or("");
                self.open_dialog(name, p.clone());
                Ok(json!({"dialog": self.dialog.as_ref().map(|d| d.name())}))
            }
            "ui.message.dismiss" => {
                self.message = None;
                Ok(Json::Null)
            }
            "ui.notice" => Ok(match &self.notice {
                Some(n) => json!({
                    "text": n.text,
                    "kind": match n.kind { NoticeKind::Warning => "warning", NoticeKind::Info => "info" },
                    "action": n.action,
                }),
                None => Json::Null,
            }),
            "ui.notice.dismiss" => {
                self.notice = None;
                Ok(Json::Null)
            }
            _ => return None,
        };
        Some(r)
    }

    /// Picks up requests commands made (dialogs, edit mode…).
    pub fn after_engine(&mut self) {
        // The requests belong to the window the command left active.
        self.follow_active_window();
        for req in self.session.take_ui_requests() {
            match req {
                UiRequest::Dialog(name, params) => self.open_dialog(&name, params),
                UiRequest::Message(m) => self.message = Some(("Monun".into(), m)),
                UiRequest::EditCell(text) => {
                    let text = text.map(|text| {
                        self.session
                            .active()
                            .map(|d| formula_locale::display_template(&text, self.ui.language.formula_locale(), &d.wb, d.wb.active_sheet()))
                            .unwrap_or(text)
                    });
                    self.begin_edit(text, false);
                }
                UiRequest::InsertName(name) => dialogs::insert_name(self, &name),
                UiRequest::OpenUrl(u) => {
                    if let Some(f) = &self.services.open_url {
                        f(&u);
                    }
                }
            }
        }
    }

    /// Whether the active sheet of the active document is protected (drives the Protect/Unprotect
    /// Sheet labels and actions).
    pub fn active_sheet_protected(&self) -> bool {
        self.session.active().and_then(|d| d.wb.active().map(|s| s.is_protected())).unwrap_or(false)
    }

    pub fn open_dialog(&mut self, name: &str, params: Json) {
        if let Err(e) = self.commit_text_box_edit() {
            self.message = Some(("Text Box".into(), clean_error(&e)));
            return;
        }
        // Every Protect Sheet entry point (ribbon menus, keytips, tab menu) becomes Unprotect Sheet
        // on a protected sheet. A sheet without a password is unprotected straight away.
        let name = if name == "protectSheet" && self.active_sheet_protected() { "unprotectSheet" } else { name };
        if name == "unprotectSheet"
            && self
                .session
                .active()
                .and_then(|d| d.wb.active())
                .and_then(|s| s.protection.as_ref())
                .is_some_and(|p| p.password.is_none() && p.legacy_password_hash.is_none())
        {
            self.run_or_alert("review.unprotectSheet", json!({}));
            return;
        }
        match name {
            "open" => {
                if let Some(start) = &self.services.open_async {
                    start();
                } else if let Some(pick) = &self.services.pick_open
                    && let Some(path) = pick()
                {
                    self.open_path(&path);
                }
            }
            "saveAs" => {
                let suggested = self.session.active().map(|d| d.display_title()).unwrap_or_else(|| "Book1".into());
                // A text file is saved as its kind (.md, .sql, .txt); a workbook as .xlsx.
                let ext = self.session.active().and_then(monun_engine::DocState::text_kind).map_or("xlsx", |k| k.extension());
                if let Some(pick) = &self.services.pick_save {
                    if let Some(path) = pick(&suggested) {
                        let path = if std::path::Path::new(&path).extension().is_none() { format!("{path}.{ext}") } else { path };
                        self.run_or_alert("file.save", json!({"path": path}));
                    }
                } else if self.services.download.is_some()
                    && let Ok(r) = self.run("file.saveBytes", json!({"format": "xlsx"}))
                    && let Some(dl) = &self.services.download
                    && let Some(b) = r.get("base64").and_then(Json::as_str).and_then(monun_engine::io::base64_decode)
                {
                    let name = r.get("name").and_then(Json::as_str).unwrap_or("Book.xlsx").to_string();
                    dl(&name, &b);
                }
            }
            // File › Options (`file.options`) and the File tab itself open the Backstage view.
            "options" => self.open_backstage(backstage::Page::Options),
            "backstage" => {
                let page = params.get("page").and_then(Json::as_str).and_then(backstage::Page::parse).unwrap_or(backstage::Page::Home);
                self.open_backstage(page);
            }
            // The macro step editor stays until it closes itself (its draft, overlays and debug
            // run go with it), so another dialog can't replace it.
            _ if self.dialog.as_ref().is_some_and(|d| d.name == "macroEditor") => {
                self.message = Some((
                    "Macro".into(),
                    "Close the macro step editor first: it keeps its window until it closes itself, so its steps and debug run are not lost.".into(),
                ));
            }
            other => self.dialog = dialogs::Dialog::open(self, other, params),
        }
    }

    /// Opens the File tab's Backstage view on `page`, committing a cell still being edited first
    /// (File actions work on the whole workbook). A draft that can't be committed keeps the sheet.
    pub fn open_backstage(&mut self, page: backstage::Page) {
        if !self.chrome.file_tab {
            return;
        }
        if !self.commit_edit(0, 0, false, false) {
            return;
        }
        match &mut self.backstage {
            Some(b) => b.page = page,
            None => self.backstage = Some(backstage::Backstage::new(page)),
        }
    }
}
