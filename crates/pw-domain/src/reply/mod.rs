//! Assistant reply concepts: control prelude extraction, sentence
//! segmentation and turn bookkeeping.

mod normalize;
mod parser;
mod sentences;
mod turn;

pub use normalize::{is_speakable, strip_emoji};
pub use parser::{ReplyControl, ReplyEvent, ReplyParser};
pub use sentences::{SentenceSplitter, is_terminator};
pub use turn::{TurnId, TurnTracker};
