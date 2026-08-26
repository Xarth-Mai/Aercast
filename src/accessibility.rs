use iced::{
    Background, Border, Color, Element, Event, Length, Rectangle, Size, Task, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
        widget::{
            Operation, Tree,
            operation::{self, Focusable, Outcome, Scrollable, focusable},
            tree,
        },
    },
    keyboard::{self, Key, key},
    touch,
    widget::{Button, Checkbox, Id, TextInput, container},
};

use super::Message;

enum Keys {
    None,
    Button(Message),
    Toggle(Message),
}

impl Keys {
    fn message(&self, key: key::Named) -> Option<Message> {
        match (self, key) {
            (Self::Button(message), key::Named::Enter | key::Named::Space) => Some(message.clone()),
            (Self::Toggle(message), key::Named::Space) => Some(message.clone()),
            _ => None,
        }
    }

    fn enabled(&self) -> bool {
        !matches!(self, Self::None)
    }
}

struct Control<'a> {
    content: Element<'a, Message>,
    keys: Keys,
    ring: Border,
}

#[derive(Default)]
struct State {
    focused: bool,
}

struct RevealFocused {
    target: Id,
    scroll: Option<(Rectangle, Vector)>,
    focused: Option<Rectangle>,
}

struct FocusAt {
    target: Option<usize>,
    current: usize,
}

impl Operation<Message> for FocusAt {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Message>)) {
        operate(self);
    }

    fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if self.target == Some(self.current) {
            state.focus();
        } else {
            state.unfocus();
        }
        self.current += 1;
    }
}

fn focus_target(count: focusable::Count, previous: bool) -> Option<usize> {
    if count.total == 0 {
        return None;
    }
    Some(match count.focused {
        Some(current) if previous => (current + count.total - 1) % count.total,
        Some(current) => (current + 1) % count.total,
        None if previous => count.total - 1,
        None => 0,
    })
}

fn next_focus(count: focusable::Count) -> FocusAt {
    FocusAt {
        target: focus_target(count, false),
        current: 0,
    }
}

fn previous_focus(count: focusable::Count) -> FocusAt {
    FocusAt {
        target: focus_target(count, true),
        current: 0,
    }
}

impl Operation<f32> for RevealFocused {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<f32>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        if id == Some(&self.target) {
            self.scroll = Some((bounds, translation));
            self.focused = None;
        }
    }

    fn focusable(&mut self, _id: Option<&Id>, bounds: Rectangle, state: &mut dyn Focusable) {
        if self.scroll.is_some() && state.is_focused() {
            self.focused = Some(bounds);
        }
    }

    fn finish(&self) -> Outcome<f32> {
        match (self.scroll, self.focused) {
            (Some((viewport, translation)), Some(focused)) => {
                Outcome::Some(scroll_delta(viewport, translation, focused))
            }
            _ => Outcome::None,
        }
    }
}

fn scroll_delta(viewport: Rectangle, translation: Vector, focused: Rectangle) -> f32 {
    const MARGIN: f32 = 8.0;
    let top = focused.y - translation.y;
    let bottom = top + focused.height;
    if top < viewport.y + MARGIN {
        top - viewport.y - MARGIN
    } else if bottom > viewport.y + viewport.height - MARGIN {
        bottom - viewport.y - viewport.height + MARGIN
    } else {
        0.0
    }
}

impl Focusable for State {
    fn is_focused(&self) -> bool {
        self.focused
    }

    fn focus(&mut self) {
        self.focused = true;
    }

    fn unfocus(&mut self) {
        self.focused = false;
    }
}

impl Widget<Message, Theme, iced::Renderer> for Control<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        if !self.keys.enabled() {
            tree.state.downcast_mut::<State>().unfocus();
        }
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if !self.keys.enabled() {
            state.unfocus();
            return;
        }
        operation.focusable(None, layout.bounds(), state);
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                layout,
                renderer,
                operation,
            );
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if !self.keys.enabled() {
            return;
        }
        let pressed_inside = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                Some(cursor.is_over(layout.bounds()))
            }
            _ => None,
        };
        let state = tree.state.downcast_mut::<State>();
        if let Some(inside) = pressed_inside
            && state.focused != inside
        {
            state.focused = inside;
            shell.request_redraw();
        }
        if state.focused
            && let Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Named(key),
                modifiers,
                repeat: false,
                ..
            }) = event
            && !modifiers.command()
            && !modifiers.alt()
            && let Some(message) = self.keys.message(*key)
        {
            shell.publish(message);
            shell.capture_event();
            return;
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        if !shell.is_event_captured()
            && pressed_inside == Some(true)
            && let Keys::Toggle(message) = &self.keys
        {
            shell.publish(message.clone());
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        if tree.state.downcast_ref::<State>().focused {
            iced::advanced::Renderer::fill_quad(
                renderer,
                renderer::Quad {
                    bounds: layout.bounds(),
                    border: self.ring,
                    ..renderer::Quad::default()
                },
                Background::Color(Color::TRANSPARENT),
            );
        }
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a> From<Control<'a>> for Element<'a, Message> {
    fn from(control: Control<'a>) -> Self {
        Self::new(control)
    }
}

pub(super) fn button<'a>(
    control: Button<'a, Message>,
    message: Option<Message>,
    ring: Border,
) -> Element<'a, Message> {
    Control {
        content: control.height(36).on_press_maybe(message.clone()).into(),
        keys: message.map_or(Keys::None, Keys::Button),
        ring,
    }
    .into()
}

pub(super) fn checkbox<'a>(
    control: Checkbox<'a, Message>,
    checked: bool,
    on_toggle: Option<impl Fn(bool) -> Message + 'a>,
    ring: Border,
) -> Element<'a, Message> {
    let message = on_toggle.as_ref().map(|on_toggle| on_toggle(!checked));
    Control {
        content: container(
            control
                .size(20)
                .width(Length::Fill)
                .on_toggle_maybe(on_toggle),
        )
        .height(36)
        .align_y(iced::alignment::Vertical::Center)
        .into(),
        keys: message.clone().map_or(Keys::None, Keys::Toggle),
        ring,
    }
    .into()
}

pub(super) fn text_input<'a>(
    control: TextInput<'a, Message>,
    enabled: bool,
) -> Element<'a, Message> {
    let content = control.padding([10, 12]);
    if enabled {
        content.into()
    } else {
        Control {
            content: content.into(),
            keys: Keys::None,
            ring: Border::default(),
        }
        .into()
    }
}

pub(super) fn move_focus(previous: bool) -> Task<Message> {
    if previous {
        iced::advanced::widget::operate(operation::then(focusable::count(), previous_focus))
    } else {
        iced::advanced::widget::operate(operation::then(focusable::count(), next_focus))
    }
}

pub(super) fn reveal_focused(id: Id) -> Task<Message> {
    iced::advanced::widget::operate(RevealFocused {
        target: id,
        scroll: None,
        focused: None,
    })
    .map(Message::RevealFocus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_keys_focus_wrap_and_scroll_reveal() {
        let button = Keys::Button(Message::Start);
        assert!(matches!(
            button.message(key::Named::Enter),
            Some(Message::Start)
        ));
        assert!(matches!(
            button.message(key::Named::Space),
            Some(Message::Start)
        ));
        let toggle = Keys::Toggle(Message::SystemAudio(true));
        assert!(matches!(
            toggle.message(key::Named::Space),
            Some(Message::SystemAudio(true))
        ));
        assert!(toggle.message(key::Named::Enter).is_none());
        assert_eq!(
            focus_target(
                focusable::Count {
                    focused: Some(2),
                    total: 3,
                },
                false,
            ),
            Some(0)
        );
        assert_eq!(
            focus_target(
                focusable::Count {
                    focused: Some(0),
                    total: 3,
                },
                true,
            ),
            Some(2)
        );
        let viewport = Rectangle {
            x: 0.0,
            y: 100.0,
            width: 400.0,
            height: 200.0,
        };
        let focused = Rectangle {
            x: 0.0,
            y: 340.0,
            width: 100.0,
            height: 36.0,
        };
        assert_eq!(
            scroll_delta(viewport, Vector::new(0.0, 50.0), focused),
            34.0
        );
        assert_eq!(
            scroll_delta(viewport, Vector::new(0.0, 250.0), focused),
            -18.0
        );
        assert_eq!(
            scroll_delta(viewport, Vector::new(0.0, 140.0), focused),
            0.0
        );
    }
}
