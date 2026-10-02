//! Sessions and undo management (plan.md §7.1, §14).
//!
//! [`DocumentSession`] bundles a [`Document`] with its undo history and a
//! generation counter for dirty tracking. This is the boundary for future
//! multi-document/tabs support.

use crate::commands::Command;
use crate::model::{Document, DocumentSource};

/// Undo/redo stacks of executed commands.
#[derive(Default)]
pub struct UndoManager {
    undo_stack: Vec<Box<dyn Command>>,
    redo_stack: Vec<Box<dyn Command>>,
}

impl UndoManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies `command` to `doc`, pushes it on the undo stack, and clears
    /// the redo stack (a new action invalidates the redo history).
    pub fn execute(&mut self, mut command: Box<dyn Command>, doc: &mut Document) {
        command.apply(doc);
        self.undo_stack.push(command);
        self.redo_stack.clear();
    }

    pub fn undo(&mut self, doc: &mut Document) -> bool {
        let mut command = match self.undo_stack.pop() {
            Some(command) => command,
            None => return false,
        };
        command.undo(doc);
        self.redo_stack.push(command);
        true
    }

    pub fn redo(&mut self, doc: &mut Document) -> bool {
        let mut command = match self.redo_stack.pop() {
            Some(command) => command,
            None => return false,
        };
        command.apply(doc);
        self.undo_stack.push(command);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Description of the next undo, e.g. "Move object" for a menu entry.
    pub fn next_undo_description(&self) -> Option<String> {
        self.undo_stack.last().map(|command| command.describe())
    }

    pub fn next_redo_description(&self) -> Option<String> {
        self.redo_stack.last().map(|command| command.describe())
    }
}

/// An open document plus editing history and dirty state.
///
/// Dirty tracking uses generation arithmetic: every executed command steps
/// the generation forward, every undo steps it back, so undoing back to the
/// last save returns the session to clean — the same content, same state.
pub struct DocumentSession {
    document: Document,
    undo: UndoManager,
    generation: u64,
    saved_generation: u64,
}

impl DocumentSession {
    pub fn new(document: Document) -> Self {
        Self {
            document,
            undo: UndoManager::new(),
            generation: 0,
            saved_generation: 0,
        }
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn undo_manager(&self) -> &UndoManager {
        &self.undo
    }

    pub fn source(&self) -> &DocumentSource {
        self.document.source()
    }

    pub fn execute(&mut self, command: Box<dyn Command>) {
        self.undo.execute(command, &mut self.document);
        self.generation += 1;
    }

    pub fn undo(&mut self) -> bool {
        if self.undo.undo(&mut self.document) {
            self.generation = self.generation.saturating_sub(1);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if self.undo.redo(&mut self.document) {
            self.generation += 1;
            true
        } else {
            false
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.generation != self.saved_generation
    }

    /// Called after a successful save/export to this session's output.
    pub fn mark_saved(&mut self) {
        self.saved_generation = self.generation;
    }
}
