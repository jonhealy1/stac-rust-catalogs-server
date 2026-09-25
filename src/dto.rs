// src/dto.rs
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
#[serde(untagged)]
pub enum CreateOrLinkPayload<T> {
    /// Mode A: Complete STAC Resource payload (must be first to avoid swallowing full resources)
    FullResource(T),
    /// Mode B: Minimal reference object containing only an ID
    LinkReference { id: String },
}
