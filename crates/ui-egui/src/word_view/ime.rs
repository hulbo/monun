//! Input-method composition in a Word tab (Hangul above all), as WordCraft's own canvas does it.
//!
//! While the document has the keyboard, every frame tells the platform where the caret is and
//! what committed text surrounds it, so the candidate window sits at the caret. A composition in
//! progress (pre-edit) isn't in the document: it is drawn at the caret until committed. A commit
//! is typed through the engine's IME entry, and when the change it made is exactly the text the
//! input method expects, a receipt says so: otherwise the platform would restart the composition
//! and drop the syllable being formed (typing 한글 would lose ㄱ after 한 was committed).

use egui::{ImeContextTracker, ImeMutationStamp, ImeObservation, ImeTarget, Rect, ViewportId};
use serde_json::json;
use wordcraft_engine::doc::Pos;
use wordcraft_engine::doc::ime_text::{ImeWindow, MAX_IME_TEXT_BYTES};
use wordcraft_engine::{ImeEditOutcome, Session};

/// The largest commit typed at once: real ones are a syllable or a word.
const MAX_COMMIT_BYTES: usize = 64 * 1024;

/// One Word tab's input-method state.
#[derive(Default)]
pub(super) struct Ime {
    tracker: ImeContextTracker,
    viewport: Option<ViewportId>,
    /// The composition in progress, drawn at the caret.
    pub preedit: String,
    /// The surrounding text last read, for an engine revision and selection.
    window: Option<(u64, Pos, Pos, Option<ImeWindow>)>,
}

impl Ime {
    /// The committed text around the caret, read again only after an edit or a caret move.
    fn window(&mut self, s: &Session) -> Option<ImeWindow> {
        let (rev, cursor, anchor) = (s.rev(), s.sel.focus.clone(), s.sel.anchor.clone());
        if let Some((r, c, a, w)) = &self.window
            && *r == rev
            && *c == cursor
            && *a == anchor
        {
            return w.clone();
        }
        let window = s.doc.ime_surrounding(&cursor, &anchor, MAX_IME_TEXT_BYTES);
        self.window = Some((rev, cursor, anchor, window.clone()));
        window
    }

    /// The stamp of the committed text around the caret, before or after an input-method edit.
    fn stamp(&mut self, s: &mut Session) -> ImeMutationStamp {
        match self.window(s) {
            Some(w) => ImeMutationStamp::from_committed(&w.text, w.cursor, w.anchor, None),
            None => match s.ime_state_revision() {
                Some(rev) => ImeMutationStamp::unavailable_at_revision(rev),
                None => ImeMutationStamp::unavailable(),
            },
        }
    }

    /// Says to the platform, at the end of the frame, where the caret is (`geometry`: the text
    /// area and the caret, on the screen) while the document has the keyboard, or that it hasn't.
    pub fn observe(&mut self, s: &mut Session, ctx: &egui::Context, geometry: Option<(Rect, Rect)>) {
        let viewport = ctx.viewport_id();
        if s.take_pending_ime_application_change() {
            self.note_application_change(s, ctx);
        }
        if self.viewport != Some(viewport) {
            if let Some(old) = self.viewport {
                self.tracker.end_viewport(old);
            }
            self.viewport = Some(viewport);
        }
        let (frame, pass) = (ctx.cumulative_frame_nr_for(viewport), ctx.cumulative_pass_nr_for(viewport));
        let update = if geometry.is_some() {
            let observation = ImeObservation::new(frame, pass, target(s));
            let observation = match self.window(s) {
                Some(w) => {
                    let committed = Some(egui::CommittedText::new(&w.text, w.cursor, w.anchor));
                    return self.publish(ctx, viewport, observation.committed(committed), geometry);
                }
                None => match s.ime_state_revision() {
                    Some(rev) => observation.unavailable_revision(rev),
                    None => observation,
                },
            };
            self.tracker.observe(viewport, Some(observation))
        } else {
            self.preedit.clear();
            self.tracker.observe(viewport, None)
        };
        set_output(ctx, update, geometry);
    }

    fn publish(&mut self, ctx: &egui::Context, viewport: ViewportId, observation: ImeObservation<'_>, geometry: Option<(Rect, Rect)>) {
        let update = self.tracker.observe(viewport, Some(observation));
        set_output(ctx, update, geometry);
    }

    /// Types an input method's commit, with the receipt that keeps the composition going.
    pub fn commit(&mut self, s: &mut Session, ctx: &egui::Context, text: &str) {
        self.preedit.clear();
        if text.is_empty() || text.len() > MAX_COMMIT_BYTES {
            return;
        }
        let before = self.stamp(s);
        if s.run_ime("text.insert", &json!({"text": text})).is_err() {
            return;
        }
        let after = self.stamp(s);
        let outcome = s.ime_edit_outcome();
        if s.take_pending_ime_application_change() {
            self.note_application_change(s, ctx);
            return;
        }
        match classify(outcome, &before, &after) {
            Proof::NoMutation => {}
            Proof::Unproven => self.note_application_change(s, ctx),
            Proof::Certified => {
                let viewport = ctx.viewport_id();
                let frame = ctx.cumulative_frame_nr_for(viewport);
                self.tracker.note_input_method_apply(viewport, frame, target(s), before, after);
            }
        }
    }

    fn note_application_change(&mut self, s: &Session, ctx: &egui::Context) {
        let viewport = ctx.viewport_id();
        let frame = ctx.cumulative_frame_nr_for(viewport);
        self.tracker.note_application_change(viewport, frame, target(s));
    }
}

/// The platform's descriptor of the editor that has the keyboard, for this frame.
fn set_output(ctx: &egui::Context, update: Option<egui::ImeContextUpdate>, geometry: Option<(Rect, Rect)>) {
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

/// Who owns the composition: the document's body. A body replaced under the same document (Undo,
/// Redo) moves on the epoch, so the platform drops a composition from before.
fn target(s: &Session) -> ImeTarget {
    ImeTarget::new(egui::Id::unique(("monun.word.ime", s.document_id(), s.sel.focus.story)), s.ime_body_epoch())
}

/// What an input-method edit proves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Proof {
    NoMutation,
    Unproven,
    Certified,
}

/// Only an untransformed content change whose stamps show the change is certified.
fn classify(outcome: ImeEditOutcome, before: &ImeMutationStamp, after: &ImeMutationStamp) -> Proof {
    if !outcome.classified {
        return if outcome.state_changed { Proof::Unproven } else { Proof::NoMutation };
    }
    if outcome.transformed || (outcome.state_changed && !outcome.content_changed) {
        return Proof::Unproven;
    }
    if outcome.content_changed {
        return if transition_proven(before, after) { Proof::Certified } else { Proof::Unproven };
    }
    if before == after { Proof::NoMutation } else { Proof::Unproven }
}

/// Whether two stamps show a change: other surrounding text, a later unavailable revision, or a
/// change between the two kinds whose unavailable side has a revision.
fn transition_proven(before: &ImeMutationStamp, after: &ImeMutationStamp) -> bool {
    use egui::output::IMESurroundingText::{Available, Unavailable};
    match (before.surrounding_text(), after.surrounding_text()) {
        (Available(a), Available(b)) => a != b,
        (Unavailable, Unavailable) => matches!((before.unavailable_revision(), after.unavailable_revision()), (Some(a), Some(b)) if a < b),
        (Unavailable, Available(_)) => before.unavailable_revision().is_some(),
        (Available(_), Unavailable) => after.unavailable_revision().is_some(),
    }
}
