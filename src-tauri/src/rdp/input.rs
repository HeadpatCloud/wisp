use ironrdp_input::{
    synchronize_event, Database, MouseButton, MousePosition, Operation, Scancode, WheelRotations,
};
use ironrdp_pdu::input::fast_path::{FastPathInputEvent, KeyboardFlags};

const BUTTONS: [(u16, MouseButton); 5] = [
    (0x01, MouseButton::Left),
    (0x02, MouseButton::Middle),
    (0x04, MouseButton::Right),
    (0x80, MouseButton::X1),
    (0x100, MouseButton::X2),
];

// (bit, vertical, units)
const WHEELS: [(u16, bool, i16); 4] =
    [(0x08, true, 120), (0x10, true, -120), (0x20, false, -120), (0x40, false, 120)];

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Key { down: bool, code: u8, extended: bool },
    Unicode { down: bool, codepoint: u32 },
    Pointer { buttons: u16, x: u16, y: u16 },
    Locks { caps: bool, num: bool, scroll: bool },
}

pub(crate) struct Translator {
    database: Database,
    buttons: u16,
}

impl Translator {
    pub fn new() -> Self {
        Translator { database: Database::new(), buttons: 0 }
    }

    pub fn apply(&mut self, input: Input) -> Vec<FastPathInputEvent> {
        let operations = match input {
            Input::Key { down, code, extended } => {
                let key = Scancode::from_u8(extended, code);
                // The database turns a second press into release and press, which drops a held
                // modifier.
                if down && self.database.is_key_pressed(key) {
                    let flags =
                        if extended { KeyboardFlags::EXTENDED } else { KeyboardFlags::empty() };
                    return vec![FastPathInputEvent::KeyboardEvent(flags, code)];
                }
                vec![if down { Operation::KeyPressed(key) } else { Operation::KeyReleased(key) }]
            }
            Input::Unicode { down, codepoint } => match char::from_u32(codepoint) {
                Some(c) if down && self.database.is_unicode_key_pressed(c) => {
                    let mut units = [0u16; 2];
                    return c
                        .encode_utf16(&mut units)
                        .iter()
                        .map(|unit| {
                            FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::empty(), *unit)
                        })
                        .collect();
                }
                Some(c) if down => vec![Operation::UnicodeKeyPressed(c)],
                Some(c) => vec![Operation::UnicodeKeyReleased(c)],
                None => Vec::new(),
            },
            Input::Locks { caps, num, scroll } => {
                return vec![synchronize_event(scroll, num, caps, false)];
            }
            Input::Pointer { buttons, x, y } => {
                let changed = self.buttons ^ buttons;
                let scrolled = buttons & !self.buttons;
                self.buttons = buttons;
                let mut operations = vec![Operation::MouseMove(MousePosition { x, y })];
                for (bit, button) in BUTTONS {
                    if changed & bit != 0 {
                        operations.push(if buttons & bit != 0 {
                            Operation::MouseButtonPressed(button)
                        } else {
                            Operation::MouseButtonReleased(button)
                        });
                    }
                }
                for (bit, is_vertical, rotation_units) in WHEELS {
                    if scrolled & bit != 0 {
                        operations.push(Operation::WheelRotations(WheelRotations {
                            is_vertical,
                            rotation_units,
                        }));
                    }
                }
                operations
            }
        };
        self.database.apply(operations).into_vec()
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_pdu::input::fast_path::SynchronizeFlags;
    use ironrdp_pdu::input::mouse::PointerFlags;
    use ironrdp_pdu::input::mouse_x::PointerXFlags;
    use ironrdp_pdu::input::{MousePdu, MouseXPdu};

    use super::*;

    fn pointer(buttons: u16, x: u16, y: u16) -> Input {
        Input::Pointer { buttons, x, y }
    }

    fn at(x: u16, y: u16, flags: PointerFlags) -> FastPathInputEvent {
        FastPathInputEvent::MouseEvent(MousePdu {
            flags,
            number_of_wheel_rotation_units: 0,
            x_position: x,
            y_position: y,
        })
    }

    fn extra(x: u16, y: u16, flags: PointerXFlags) -> FastPathInputEvent {
        FastPathInputEvent::MouseEventEx(MouseXPdu { flags, x_position: x, y_position: y })
    }

    fn wheel(x: u16, y: u16, vertical: bool, units: i16) -> FastPathInputEvent {
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: if vertical {
                PointerFlags::VERTICAL_WHEEL
            } else {
                PointerFlags::HORIZONTAL_WHEEL
            },
            number_of_wheel_rotation_units: units,
            x_position: x,
            y_position: y,
        })
    }

    #[test]
    fn each_button_goes_down_and_up_after_the_move() {
        let down = PointerFlags::DOWN;
        let cases = [
            (
                0x01,
                at(10, 20, down | PointerFlags::LEFT_BUTTON),
                at(10, 20, PointerFlags::LEFT_BUTTON),
            ),
            (
                0x02,
                at(10, 20, down | PointerFlags::MIDDLE_BUTTON_OR_WHEEL),
                at(10, 20, PointerFlags::MIDDLE_BUTTON_OR_WHEEL),
            ),
            (
                0x04,
                at(10, 20, down | PointerFlags::RIGHT_BUTTON),
                at(10, 20, PointerFlags::RIGHT_BUTTON),
            ),
            (
                0x80,
                extra(10, 20, PointerXFlags::DOWN | PointerXFlags::BUTTON1),
                extra(10, 20, PointerXFlags::BUTTON1),
            ),
            (
                0x100,
                extra(10, 20, PointerXFlags::DOWN | PointerXFlags::BUTTON2),
                extra(10, 20, PointerXFlags::BUTTON2),
            ),
        ];
        for (mask, pressed, released) in cases {
            let mut translator = Translator::new();
            let moved = at(10, 20, PointerFlags::MOVE);
            assert_eq!(translator.apply(pointer(mask, 10, 20)), [moved, pressed], "{mask:#x}");
            assert_eq!(translator.apply(pointer(0, 10, 20)), [released], "{mask:#x}");
        }
    }

    #[test]
    fn two_buttons_changing_at_once_come_in_button_order() {
        let mut translator = Translator::new();
        translator.apply(pointer(0x01, 5, 5));
        let events = translator.apply(pointer(0x04, 5, 5));
        assert_eq!(
            events,
            [
                at(5, 5, PointerFlags::LEFT_BUTTON),
                at(5, 5, PointerFlags::DOWN | PointerFlags::RIGHT_BUTTON),
            ]
        );
        let events = translator.apply(pointer(0x03, 6, 7));
        assert_eq!(
            events,
            [
                at(6, 7, PointerFlags::MOVE),
                at(6, 7, PointerFlags::DOWN | PointerFlags::LEFT_BUTTON),
                at(6, 7, PointerFlags::DOWN | PointerFlags::MIDDLE_BUTTON_OR_WHEEL),
                at(6, 7, PointerFlags::RIGHT_BUTTON),
            ]
        );
    }

    #[test]
    fn a_wheel_bit_held_over_three_events_scrolls_once() {
        let mut translator = Translator::new();
        assert_eq!(translator.apply(pointer(0x08, 0, 0)), [wheel(0, 0, true, 120)]);
        assert_eq!(translator.apply(pointer(0x08, 1, 0)), [at(1, 0, PointerFlags::MOVE)]);
        assert_eq!(translator.apply(pointer(0x08, 1, 1)), [at(1, 1, PointerFlags::MOVE)]);
        assert_eq!(translator.apply(pointer(0, 1, 1)), []);
        assert_eq!(translator.apply(pointer(0x08, 1, 1)), [wheel(1, 1, true, 120)]);
    }

    #[test]
    fn the_wheel_directions_use_120_units() {
        for (mask, vertical, units) in
            [(0x08, true, 120), (0x10, true, -120), (0x20, false, -120), (0x40, false, 120)]
        {
            let mut translator = Translator::new();
            assert_eq!(
                translator.apply(pointer(mask, 0, 0)),
                [wheel(0, 0, vertical, units)],
                "{mask:#x}"
            );
        }
    }

    #[test]
    fn keys_carry_the_extended_flag() {
        let mut translator = Translator::new();
        let key = |down, code, extended| Input::Key { down, code, extended };
        assert_eq!(
            translator.apply(key(true, 0x1e, false)),
            [FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1e)]
        );
        assert_eq!(
            translator.apply(key(false, 0x1e, false)),
            [FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x1e)]
        );
        assert_eq!(
            translator.apply(key(true, 0x48, true)),
            [FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED, 0x48)]
        );
        assert_eq!(
            translator.apply(key(false, 0x48, true)),
            [FastPathInputEvent::KeyboardEvent(
                KeyboardFlags::EXTENDED | KeyboardFlags::RELEASE,
                0x48
            )]
        );
    }

    #[test]
    fn unicode_is_sent_per_utf16_unit_and_invalid_values_are_dropped() {
        let mut translator = Translator::new();
        let unicode = |down, codepoint| Input::Unicode { down, codepoint };
        assert_eq!(
            translator.apply(unicode(true, 0xe9)),
            [FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::empty(), 0xe9)]
        );
        assert_eq!(
            translator.apply(unicode(false, 0xe9)),
            [FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::RELEASE, 0xe9)]
        );
        assert_eq!(translator.apply(unicode(true, 0x1f600)).len(), 2);
        assert_eq!(translator.apply(unicode(true, 0xd800)), []);
        assert_eq!(translator.apply(unicode(false, 0x110000)), []);
        assert_eq!(translator.apply(unicode(true, u32::MAX)), []);
    }

    #[test]
    fn a_held_key_repeats_as_a_press_only() {
        let mut translator = Translator::new();
        let key = |down, code, extended| Input::Key { down, code, extended };
        for (code, extended, flags) in [
            (0x1e, false, KeyboardFlags::empty()),
            (0x48, true, KeyboardFlags::EXTENDED),
        ] {
            let press = FastPathInputEvent::KeyboardEvent(flags, code);
            let release = FastPathInputEvent::KeyboardEvent(flags | KeyboardFlags::RELEASE, code);
            assert_eq!(translator.apply(key(true, code, extended)), [press.clone()]);
            for _ in 0..3 {
                assert_eq!(translator.apply(key(true, code, extended)), [press.clone()]);
            }
            assert_eq!(translator.apply(key(false, code, extended)), [release]);
            assert_eq!(translator.apply(key(false, code, extended)), []);
            assert_eq!(translator.apply(key(true, code, extended)), [press]);
        }
    }

    #[test]
    fn a_held_unicode_key_repeats_as_a_press_only() {
        let mut translator = Translator::new();
        let unicode = |down, codepoint| Input::Unicode { down, codepoint };
        let press = |unit| FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::empty(), unit);
        let release =
            |unit| FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::RELEASE, unit);
        assert_eq!(translator.apply(unicode(true, 0xe9)), [press(0xe9)]);
        assert_eq!(translator.apply(unicode(true, 0xe9)), [press(0xe9)]);
        assert_eq!(translator.apply(unicode(false, 0xe9)), [release(0xe9)]);

        let (high, low) = (0xd83e, 0xdd80);
        assert_eq!(translator.apply(unicode(true, 0x1f980)), [press(high), press(low)]);
        assert_eq!(translator.apply(unicode(true, 0x1f980)), [press(high), press(low)]);
        assert_eq!(translator.apply(unicode(false, 0x1f980)), [release(high), release(low)]);
    }

    #[test]
    fn a_held_modifier_stays_down_while_a_letter_repeats() {
        let mut translator = Translator::new();
        let key = |down, code| Input::Key { down, code, extended: false };
        let (alt, tab) = (0x38, 0x0f);
        let mut wire = Vec::new();
        for input in [
            key(true, alt),
            key(true, tab),
            key(true, alt),
            key(true, tab),
            key(true, alt),
            key(false, tab),
            key(false, alt),
        ] {
            wire.extend(translator.apply(input));
        }
        let event = |flags, code| FastPathInputEvent::KeyboardEvent(flags, code);
        let none = KeyboardFlags::empty();
        let up = KeyboardFlags::RELEASE;
        assert_eq!(
            wire,
            [
                event(none, alt),
                event(none, tab),
                event(none, alt),
                event(none, tab),
                event(none, alt),
                event(up, tab),
                event(up, alt),
            ]
        );
    }

    #[test]
    fn locks_are_one_synchronize_event() {
        let mut translator = Translator::new();
        let locks = |caps, num, scroll| Input::Locks { caps, num, scroll };
        assert_eq!(
            translator.apply(locks(true, true, false)),
            [FastPathInputEvent::SyncEvent(
                SynchronizeFlags::NUM_LOCK | SynchronizeFlags::CAPS_LOCK
            )]
        );
        assert_eq!(
            translator.apply(locks(false, false, true)),
            [FastPathInputEvent::SyncEvent(SynchronizeFlags::SCROLL_LOCK)]
        );
        assert_eq!(
            translator.apply(locks(false, false, false)),
            [FastPathInputEvent::SyncEvent(SynchronizeFlags::empty())]
        );
        for (input, bits) in [
            (locks(true, false, false), 0x04),
            (locks(false, true, false), 0x02),
            (locks(false, false, true), 0x01),
        ] {
            let alone = FastPathInputEvent::SyncEvent(SynchronizeFlags::from_bits_retain(bits));
            assert_eq!(translator.apply(input), [alone], "{bits:#x}");
        }
    }

    #[test]
    fn six_hundred_key_events_come_back_as_six_hundred() {
        let mut translator = Translator::new();
        let events: Vec<_> = (0..600)
            .flat_map(|n| {
                let code = (n / 2 % 100 + 1) as u8;
                let key = Input::Key { down: n % 2 == 0, code, extended: false };
                translator.apply(key)
            })
            .collect();
        assert_eq!(events.len(), 600);
    }
}
