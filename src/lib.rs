//! Herdr Yank: a Herdr plugin that copies visible pane tokens, or opens visible URLs and local
//! files, with short keyboard hints.
//!
//! Only `cli` is public. The other modules are private, so the compiler reports dead code.

pub mod cli;
mod clipboard;
mod config;
mod herdr;
mod hints;
mod model;
mod patterns;
mod picker;
mod renderer;
#[cfg(test)]
mod test_support;
mod theme;
mod url_opener;
mod viewport;
