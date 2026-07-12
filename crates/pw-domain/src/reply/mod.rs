//! Assistant reply concepts: control prelude extraction, sentence
//! segmentation and turn bookkeeping.

mod parser;
mod sentences;
mod turn;

pub use parser::{ReplyControl, ReplyEvent, ReplyParser};
pub use sentences::SentenceSplitter;
pub use turn::{TurnId, TurnTracker};
