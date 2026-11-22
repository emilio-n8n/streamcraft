//! Demo document.
//!
//! This is deliberately small. A generated application replaces this type with
//! its own domain model and keeps the surrounding lifecycle code.

use serde::{Deserialize, Serialize};

/// The application document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// Document title.
    pub title: String,
    /// Free-form notes.
    pub notes: Vec<String>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            title: "Untitled".into(),
            notes: Vec::new(),
        }
    }
}

impl Document {
    /// Built-in demonstration content.
    #[must_use]
    pub fn sample() -> Self {
        Self {
            title: "CraftKit demo".into(),
            notes: vec![
                "This document demonstrates the CraftKit project lifecycle.".into(),
                "Every edit goes through the undo history.".into(),
                "The background job runs off the UI thread and can be cancelled.".into(),
                "Replace this document type with your application domain.".into(),
            ],
        }
    }
}
