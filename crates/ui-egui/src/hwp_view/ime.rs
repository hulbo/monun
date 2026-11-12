//! Input-method composition in a 한글 tab (Hangul above all).
//!
//! While the document has the keyboard, every frame tells the platform where the caret is, so the
//! candidate window sits at the caret. A composition in progress (pre-edit) isn't in the document:
//! it is drawn at the caret until committed. The document lives on its own thread, so the text
//! around the caret isn't at hand: the platform is told it is unavailable, at a revision this
//! keeps. A commit moves that revision on and says the change was the input method's own (a
//! receipt): otherwise the platform would restart the composition and drop the syllable being
//! formed (typing 한글 would lose ㄱ after 한 was committed). Any other change (a click, a key,
//! Undo) moves it on as the application's.

use egui::{ImeContextTracker, ImeMutationStamp, ImeObservation, ImeTarget, Rect, ViewportId};

/// One 한글 tab's input-method state.
pub(super) struct Ime {
    tracker: ImeContextTracker,
    viewport: Option<ViewportId>,
    /// Whose composition it is: the tab's document.
    owner: egui::Id,
    /// Moves on when the document is replaced under the same tab (Undo, Redo).
    epoch: u64,
    /// The revision of the text around the caret, as the platform knows it.
    rev: u64,
    /// The composition in progress, drawn at the caret.
    pub preedit: String,
}

impl Ime {
    pub fn new(uid: u64) -> Ime {
        Ime {
            tracker: ImeContextTracker::new(),
            viewport: None,
            owner: egui::Id::unique(("monun.hwp.ime", uid)),
            epoch: 0,
            rev: 0,
            preedit: String::new(),
        }
    }

    fn target(&self) -> ImeTarget {
        ImeTarget::new(self.owner, self.epoch)
    }

    /// Says to the platform, at the end of the frame, where the caret is (`geometry`: the text
    /// area and the caret, on the screen) while the document has the keyboard, or that it hasn't.
    pub fn observe(&mut self, ctx: &egui::Context, geometry: Option<(Rect, Rect)>) {
        let viewport = ctx.viewport_id();
        if self.viewport != Some(viewport) {
            if let Some(old) = self.viewport {
                self.tracker.end_viewport(old);
            }
            self.viewport = Some(viewport);
        }
        let (frame, pass) = (ctx.cumulative_frame_nr_for(viewport), ctx.cumulative_pass_nr_for(viewport));
        let update = match geometry {
            Some(_) => self.tracker.observe(viewport, Some(ImeObservation::new(frame, pass, self.target()).unavailable_revision(self.rev))),
            None => {
                self.preedit.clear();
                self.tracker.observe(viewport, None)
            }
        };
        if let (Some(update), Some((rect, cursor_rect))) = (update, geometry) {
            ctx.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    owner: update.owner,
                    context_generation: update.context_generation,
                    purpose: update.purpose,
                    rect,
                    cursor_rect,
                    should_interrupt_composition: false,
                    reset_request: None,
                    change_cause: update.change_cause,
                    surrounding_text: update.surrounding_text,
                });
            });
        }
    }

    /// An input method's commit, sent to the document: the receipt that keeps the composition
    /// going. Returns the text to type, or `None` for nothing.
    pub fn commit(&mut self, ctx: &egui::Context, text: &str) -> Option<String> {
        self.preedit.clear();
        if text.is_empty() {
            return None;
        }
        let before = ImeMutationStamp::unavailable_at_revision(self.rev);
        self.rev += 1;
        let after = ImeMutationStamp::unavailable_at_revision(self.rev);
        let viewport = ctx.viewport_id();
        let frame = ctx.cumulative_frame_nr_for(viewport);
        self.tracker.note_input_method_apply(viewport, frame, self.target(), before, after);
        Some(text.to_owned())
    }

    /// The document changed other than by the input method (typing, a key, a click, a paste):
    /// the platform reads it again.
    pub fn changed(&mut self, ctx: &egui::Context, replaced: bool) {
        self.rev += 1;
        let viewport = ctx.viewport_id();
        let frame = ctx.cumulative_frame_nr_for(viewport);
        self.tracker.note_application_change(viewport, frame, self.target());
        if replaced {
            self.epoch += 1;
        }
    }
}
