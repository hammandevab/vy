//! One event on its way through the components of a page.

use crate::event::Event;

/// What a component did with an event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Response {
    /// Nothing visible changed.
    #[default]
    Nothing,
    /// Only the look changed (focus, caret, scroll, hover of a drag).
    Redraw,
    /// The value changed: text edited, box toggled, option or row
    /// selected, slider moved.
    Changed,
    /// Activated: a button pressed, Enter in a one-line field, a list row
    /// opened.
    Activated,
}

impl Response {
    /// True when the page should be drawn again.
    pub fn redraw(self) -> bool {
        self != Response::Nothing
    }

    /// True when the component's value changed.
    pub fn changed(self) -> bool {
        self == Response::Changed
    }

    /// True when the component was activated.
    pub fn activated(self) -> bool {
        self == Response::Activated
    }
}

/// One event as `App::event` hands it to the components.
///
/// ```
/// # use vy_sdk::prelude::*;
/// # use vy_sdk::ui::Input;
/// # fn handle(event: Event<'_>) -> bool {
/// let mut ui = Input::new(event);
/// // self.name.event(&mut ui, l.name, &self.theme); ... every component
/// if let Some(Event::KeyDown { code: vy_sdk::event::key::ESCAPE, .. }) = ui.unhandled() {
///     // the page's own keys, for what no component took
/// }
/// ui.redraw()
/// # }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    event: Event<'a>,
    taken: bool,
    redraw: bool,
}

impl<'a> Input<'a> {
    /// Wrap the event `App::event` got.
    pub fn new(event: Event<'a>) -> Self {
        Self {
            event,
            taken: false,
            redraw: false,
        }
    }

    /// The event, whether or not a component took it.
    pub fn event(&self) -> Event<'a> {
        self.event
    }

    /// True when a component used the event; components after it ignore
    /// it. A `FOCUS` event is never taken.
    pub fn taken(&self) -> bool {
        self.taken
    }

    /// The event unless a component took it: what a component or the page
    /// itself may still act on.
    pub fn unhandled(&self) -> Option<Event<'a>> {
        (!self.taken).then_some(self.event)
    }

    /// Mark the event used, so the components after this one ignore it.
    /// A page's own control calls this too. Does nothing for a `FOCUS`
    /// event, which every component must see (SPEC-10.3.1).
    pub fn take(&mut self) {
        if !matches!(self.event, Event::Focus { .. }) {
            self.taken = true;
        }
    }

    /// Record a component's response; returns it.
    pub fn respond(&mut self, r: Response) -> Response {
        self.redraw |= r.redraw();
        r
    }

    /// Ask for a frame for a change the page made itself.
    pub fn set_redraw(&mut self) {
        self.redraw = true;
    }

    /// What `App::event` returns: true when a component or the page asked
    /// for a redraw (SPEC-5.1.3).
    pub fn redraw(&self) -> bool {
        self.redraw
    }
}
