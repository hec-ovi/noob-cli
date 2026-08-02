//! One box per pane widget: each paints one view from the frame, through
//! the layout contract, and nothing else. Adding a widget is adding a
//! folder here.

pub mod activity;
pub mod agent;
pub mod agents;
pub mod context;
pub mod files;
pub mod gauges;
pub mod output;
pub mod plan;
pub mod popup;

/// How wide the label beside a reading is, in columns: the widest label the
/// monitors carry with a space after it. Shared by the meters and the context
/// pane, which is why it sits with the widgets rather than inside either.
pub(crate) const LABEL_COLUMNS: usize = 9;
