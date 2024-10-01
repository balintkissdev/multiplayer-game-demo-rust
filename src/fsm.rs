#[derive(Clone, Copy)]
pub enum SessionMode {
    CreateServer,
    ConnectAsClientOnly,
}

pub enum State {
    Menu,
    Connecting {
        server_address: String,
        session_mode: SessionMode,
    },
    Playing,
    Disconnected,
    QuitDialog,
    Quit,
}

/// In-house Finite State Machine for transitioning between menus and application states. It
/// implements a Pushdown Automata to add dialogs like the Quit dialog and pop back to previous
/// state like a stack. Basically just a thin wrapper around a Vec, which is recommended for stack
/// data structures by https://doc.rust-lang.org/std/collections/index.html#use-a-vec-when
///
/// There are third-party crates like rust_fsm, but not necessary to include too much crates.
pub struct StateMachine {
    state_stack: Vec<State>,
}

impl StateMachine {
    pub fn new() -> Self {
        Self {
            state_stack: Vec::new(),
        }
    }

    pub fn push(&mut self, state: State) {
        self.state_stack.push(state);
    }

    pub fn pop(&mut self) {
        self.state_stack.pop();
    }

    pub fn change(&mut self, state: State) {
        self.state_stack.clear();
        self.push(state);
    }

    pub fn peek(&self) -> Option<&State> {
        self.state_stack.last()
    }

    pub fn peek_mut(&mut self) -> Option<&mut State> {
        self.state_stack.last_mut()
    }
}
