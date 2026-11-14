//! Lumbridge Voice: the speech-to-text HTTP front.
//!
//! It serves the API of the Python front it replaces (`front-py/speech_front.py`,
//! described in `docs/API.md`). The model is behind the [`adapter::Transcriber`] trait.

pub mod adapter;
pub mod config;
pub mod server;
pub mod speak;
pub mod text;
pub mod vllm;
pub mod wav;
