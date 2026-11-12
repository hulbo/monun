//! What a 한글 tab asks its document's thread, and what the thread answers.
//!
//! The thread owns the document (rhwp's `HwpDocument`) and does everything that touches it in
//! the order asked; the tab never waits for it, but for Save, which takes milliseconds. After each
//! run of requests it answers with the document's [`State`]: its revision, pages, caret, selection
//! and formatting there. Positions on a page are in points from its top left corner.

use std::sync::{Arc, mpsc};

/// A caret move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Move {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
    PageUp,
    PageDown,
}

/// What a press on a page does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Point {
    /// Puts the caret there, or with `extend` selects to there.
    Caret { extend: bool },
    /// Selects the word there.
    Word,
    /// Selects the paragraph there.
    Paragraph,
}

/// Character formatting turned on or off over the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Toggle {
    Bold,
    Italic,
    Underline,
    Strike,
}

/// A table edit at the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TableOp {
    Insert { rows: u16, cols: u16 },
    RowAbove,
    RowBelow,
    ColumnLeft,
    ColumnRight,
    DeleteRow,
    DeleteColumn,
}

/// A file made of the document: its bytes, and what it couldn't keep (rhwp's content-loss report;
/// empty when it kept everything it knows of).
pub(crate) struct Export {
    pub bytes: Vec<u8>,
    pub lost: Vec<String>,
}

/// A request to the document's thread.
pub(crate) enum Req {
    /// Typed text, at the caret (over the selection).
    Type(String),
    Move {
        to: Move,
        extend: bool,
    },
    /// Backspace (`back`) or Delete, a character or (`word`) a word.
    Erase {
        back: bool,
        word: bool,
    },
    Enter,
    /// Tab: the next (or previous) table cell; a tab character outside tables.
    Tab {
        back: bool,
    },
    /// A press on `page` at `x`, `y` (points).
    Point {
        page: usize,
        x: f32,
        y: f32,
        how: Point,
    },
    SelectAll,
    /// The selection dropped, the caret at its focus.
    Collapse,
    Toggle(Toggle),
    Font(String),
    Size(f32),
    /// The size a step larger (or smaller).
    Grow(bool),
    /// Text colour, `0xRRGGBB`.
    Color(u32),
    /// Paragraph alignment: `left`, `center`, `right` or `justify`.
    Align(&'static str),
    Table(TableOp),
    Undo,
    Redo,
    /// Looks for `text` from the selection on (or back), selecting it; `again` from its end (its
    /// start: `text` just typed into the find field).
    Find {
        text: String,
        forward: bool,
        again: bool,
    },
    Replace {
        text: String,
        with: String,
    },
    ReplaceAll {
        text: String,
        with: String,
    },
    Copy {
        cut: bool,
    },
    Paste {
        text: String,
        html: Option<String>,
    },
    /// The caret to the `n`th entry of the outline.
    Goto(usize),
    /// The SVG of these pages, as they are now.
    Pages(Vec<usize>),
    /// The outline (the headings).
    Outline,
    /// The document as a file (`.hwpx` or `.hwp`), answered on its own channel.
    Export {
        hwpx: bool,
        answer: mpsc::Sender<Result<Export, String>>,
    },
    /// Every page's SVG, for printing, answered on its own channel.
    PrintPages {
        answer: mpsc::Sender<Result<Vec<String>, String>>,
    },
    /// The document was saved: it has no unsaved changes.
    Saved,
    /// The document's text (for checks), answered on its own channel.
    Text {
        answer: mpsc::Sender<String>,
    },
}

impl Req {
    /// Whether the request changes the document (rather than the caret, the selection or what is
    /// shown).
    pub fn edits(&self) -> bool {
        match self {
            Req::Type(_)
            | Req::Erase { .. }
            | Req::Enter
            | Req::Tab { .. }
            | Req::Toggle(_)
            | Req::Font(_)
            | Req::Size(_)
            | Req::Grow(_)
            | Req::Color(_)
            | Req::Align(_)
            | Req::Table(_)
            | Req::Undo
            | Req::Redo
            | Req::Replace { .. }
            | Req::ReplaceAll { .. }
            | Req::Paste { .. } => true,
            Req::Copy { cut } => *cut,
            _ => false,
        }
    }
}

/// Formatting at the caret, for the tool bar.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Format {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub size: Option<f32>,
    pub font: Option<String>,
    pub align: Option<String>,
}

/// The document as it is after a run of requests.
#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    /// Counts the changes to the document's content.
    pub rev: u64,
    /// The last request done, of those the tab sent (by number).
    pub done: u64,
    /// The pages' sizes (points).
    pub pages: Arc<Vec<[f32; 2]>>,
    /// The caret: its page, and its top and height there (`[x, y, height]`, points).
    pub caret: Option<(usize, [f32; 3])>,
    /// The selection's rectangles, by page (`[x, y, width, height]`, points).
    pub selection: Vec<(usize, [f32; 4])>,
    pub dirty: bool,
    pub format: Format,
    /// The caret is in a table cell.
    pub in_table: bool,
}

/// What the document's thread says.
pub(crate) enum Reply {
    /// The document was read (or made), and how it is.
    Ready(State),
    /// The document couldn't be read: why (`password`, `damaged` or the error).
    Failed(String),
    /// How the document is after a run of requests.
    State(State),
    /// A page's SVG, at a revision, with its fingerprint.
    Page {
        rev: u64,
        page: usize,
        key: u64,
        svg: Arc<String>,
    },
    /// What Copy (or Cut) took: its text, and its HTML where it has formatting.
    Copied {
        text: String,
        html: Option<String>,
    },
    /// What Find found: how many in all, and which one is selected.
    Found {
        count: usize,
        index: Option<usize>,
    },
    Replaced(usize),
    /// The outline at a revision: each heading's level and text.
    Outline(u64, Vec<(u8, String)>),
    /// An edit refused, and why.
    Refused(String),
}
