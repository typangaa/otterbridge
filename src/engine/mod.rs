//! Engine layer: the four execution patterns weir supports.
//!
//! Each sub-module is a pure-function (or near-pure) async entry point that
//! receives resolved [`Backend`] handles and returns [`ChatResponse`] data.
//! No config-loading or backend construction happens here; that belongs to the
//! caller (MCP handler / CLI command).

pub mod eval_loop;
pub mod fan_out;
pub mod fusion;
pub mod pipeline;
pub mod router;
