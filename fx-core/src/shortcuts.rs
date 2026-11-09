//! Keyboard shortcut chords.
//!
//! Built-in chords stay until the user changes them. The file stores only the
//! overrides, one `action=Chord` line each. A missing file stays missing and
//! reads as [`ShortcutMap::builtin`]. An empty file does not parse: loading it
//! returns [`LoadError::Parse`], not the builtin map. Callers store only when
//! there is at least one override. [`ShortcutMap::store`] of a map with no
//! overrides writes an empty string, and that empty file fails to load.
//!
//! This map does not decide frecency, themes, or eviction. Pressing Enter to
//! submit the address bar is not a shortcut here. That key applies only while
//! the address bar has focus.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

/// Win32 `VK_TAB`. Stored as a number so this crate does not link Win32.
const VK_TAB: u16 = 0x09;
/// Win32 `VK_LEFT`.
const VK_LEFT: u16 = 0x25;
/// Win32 `VK_UP`.
const VK_UP: u16 = 0x26;
/// Win32 `VK_RIGHT`.
const VK_RIGHT: u16 = 0x27;
/// Win32 `VK_DOWN`.
const VK_DOWN: u16 = 0x28;
/// Win32 `VK_F5`.
const VK_F5: u16 = 0x74;
/// Win32 `VK_F6`.
const VK_F6: u16 = 0x75;

/// A command the shell can bind to a chord.
///
/// The names used in the overrides file are the snake_case names below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Previous location. Default: Alt+Left (`VK_LEFT`, 0x25).
    Back,
    /// Next location. Default: Alt+Right (`VK_RIGHT`, 0x27).
    Forward,
    /// Focus the address bar. Default: Alt+D (0x44).
    FocusAddress,
    /// Switch the active pane. Default: F6 (0x75).
    TogglePane,
    /// Open a new tab. Default: Ctrl+T (0x54).
    NewTab,
    /// Close the current tab. Default: Ctrl+W (0x57).
    CloseTab,
    /// Select the next tab. Default: Ctrl+Tab (`VK_TAB`, 0x09).
    NextTab,
    /// Select the previous tab. Default: Ctrl+Shift+Tab (`VK_TAB`, 0x09).
    PreviousTab,
    /// Refresh the current listing. Default: F5 (0x74).
    Refresh,
}

impl Action {
    /// Enum order. [`Self::index`] matches this position.
    const ALL: [Self; 9] = [
        Self::Back,
        Self::Forward,
        Self::FocusAddress,
        Self::TogglePane,
        Self::NewTab,
        Self::CloseTab,
        Self::NextTab,
        Self::PreviousTab,
        Self::Refresh,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Back => 0,
            Self::Forward => 1,
            Self::FocusAddress => 2,
            Self::TogglePane => 3,
            Self::NewTab => 4,
            Self::CloseTab => 5,
            Self::NextTab => 6,
            Self::PreviousTab => 7,
            Self::Refresh => 8,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Back => "back",
            Self::Forward => "forward",
            Self::FocusAddress => "focus_address",
            Self::TogglePane => "toggle_pane",
            Self::NewTab => "new_tab",
            Self::CloseTab => "close_tab",
            Self::NextTab => "next_tab",
            Self::PreviousTab => "previous_tab",
            Self::Refresh => "refresh",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.name() == name)
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Ctrl, Alt, and Shift plus one virtual key.
///
/// `virtual_key` is a Win32 virtual-key code. Letters use the ASCII code of
/// the uppercase letter (`D` is `0x44`). This struct does not link Win32.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Virtual-key code, such as `0x25` for Left or `0x09` for Tab.
    pub virtual_key: u16,
}

impl fmt::Display for Chord {
    /// Canonical text: modifiers in the order Ctrl, Alt, Shift, then the key.
    ///
    /// Letters are one uppercase ASCII letter. Named keys are `Left`, `Right`,
    /// `Up`, `Down`, `Tab`, `F5`, and `F6`. Any other virtual key is written
    /// `VK` plus four uppercase hex digits. [`ShortcutMap::parse`] accepts the
    /// letter and named-key forms only.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        display_key(self.virtual_key, f)
    }
}

/// One chord per [`Action`].
///
/// Built-in chords stay in place until [`Self::rebind`] or a parsed override
/// replaces them. Two actions never share a chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutMap {
    chords: [Chord; 9],
}

impl ShortcutMap {
    /// Alt+Left, Alt+Right, Alt+D, F6, Ctrl+T, Ctrl+W, Ctrl+Tab,
    /// Ctrl+Shift+Tab, and F5, in [`Action`] enum order.
    pub fn builtin() -> Self {
        let chords = Action::ALL.map(builtin_chord);
        debug_assert!(
            !chords_collide(&chords),
            "built-in shortcut chords must be unique"
        );
        Self { chords }
    }

    /// The action bound to `chord`, if any.
    pub fn lookup(&self, chord: Chord) -> Option<Action> {
        Action::ALL
            .into_iter()
            .find(|&action| self.chord(action) == chord)
    }

    /// The chord currently bound to `action`.
    pub fn chord(&self, action: Action) -> Chord {
        self.chords[action.index()]
    }

    /// Replace this action's chord.
    ///
    /// Returns [`RebindError::ChordInUse`] when another action already has
    /// `chord`. The map is unchanged on error. Binding an action to the chord
    /// it already has succeeds.
    pub fn rebind(&mut self, action: Action, chord: Chord) -> Result<(), RebindError> {
        match self.lookup(chord) {
            Some(owner) if owner != action => return Err(RebindError::ChordInUse(owner)),
            Some(_) => return Ok(()),
            None => {}
        }
        self.chords[action.index()] = chord;
        Ok(())
    }

    /// Sparse overrides over the builtin map.
    ///
    /// One `action=Chord` per line. Action names are the snake_case names of
    /// [`Action`] and are case-sensitive. Chord words are case-insensitive.
    /// Modifiers, when present, stay in the order Ctrl, Alt, Shift, then the
    /// key. Letters are one ASCII letter. Named keys are Left, Right, Up,
    /// Down, Tab, F5, and F6.
    ///
    /// Empty or whitespace-only text is an error. A blank line is ignored.
    /// Unknown action, bad chord, duplicate action, or duplicate chord is an
    /// error. Nothing is applied when parsing fails. An override that repeats
    /// that action's builtin chord is kept and is not a duplicate.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        if text.trim().is_empty() {
            return Err(ParseError::Empty);
        }
        let mut map = Self::builtin();
        let mut seen = [false; Action::ALL.len()];
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (action, chord) = parse_line(line)?;
            let index = action.index();
            if seen[index] {
                return Err(ParseError::DuplicateAction);
            }
            seen[index] = true;
            map.chords[index] = chord;
        }
        if chords_collide(&map.chords) {
            return Err(ParseError::DuplicateChord);
        }
        Ok(map)
    }

    /// Reads overrides from `path`.
    ///
    /// A missing file returns [`Self::builtin`] and does not create the file.
    /// Any other I/O failure, or a file that does not parse, is returned.
    /// An empty file is a parse error, not the builtin map.
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(LoadError::Parse),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::builtin()),
            Err(err) => Err(LoadError::Io(err)),
        }
    }

    /// Writes one `action=Chord` line per override, each with a trailing newline.
    ///
    /// Actions stay in enum order. Chords use the canonical [`Chord`] display.
    /// When nothing differs from builtin, this writes an empty string. That
    /// empty file does not parse. Callers store only when [`Self::overrides`]
    /// is non-empty; a missing file is how the builtin map is represented.
    pub fn store(&self, path: &Path) -> io::Result<()> {
        let text: String = self
            .overrides()
            .into_iter()
            .map(|(action, chord)| format!("{action}={chord}\n"))
            .collect();
        fs::write(path, text)
    }

    /// Actions whose chord differs from builtin, in enum order.
    pub fn overrides(&self) -> Vec<(Action, Chord)> {
        let builtin = Self::builtin();
        Action::ALL
            .into_iter()
            .filter_map(|action| {
                let chord = self.chord(action);
                (chord != builtin.chord(action)).then_some((action, chord))
            })
            .collect()
    }
}

/// [`ShortcutMap::rebind`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebindError {
    /// Another action already uses this chord.
    ///
    /// The payload is the action that owns the chord.
    ChordInUse(Action),
}

impl fmt::Display for RebindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChordInUse(action) => {
                write!(f, "keyboard shortcut chord is already used by {action}")
            }
        }
    }
}

impl std::error::Error for RebindError {}

/// Shortcut text could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Empty, or only whitespace.
    Empty,
    /// A line is not `action=Chord`.
    Invalid,
    /// The action name is not one of the shortcut actions.
    UnknownAction,
    /// The chord is not modifiers plus a letter or named key.
    BadChord,
    /// The same action appears on more than one line.
    DuplicateAction,
    /// Two actions would share one chord.
    DuplicateChord,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "keyboard shortcuts are empty"),
            Self::Invalid => write!(f, "keyboard shortcut must be action=Chord"),
            Self::UnknownAction => write!(f, "unknown keyboard shortcut action"),
            Self::BadChord => write!(f, "keyboard shortcut chord is not valid"),
            Self::DuplicateAction => write!(f, "keyboard shortcut action is repeated"),
            Self::DuplicateChord => write!(f, "keyboard shortcut chord is already in use"),
        }
    }
}

impl std::error::Error for ParseError {}

/// [`ShortcutMap::load`] failed.
#[derive(Debug)]
pub enum LoadError {
    Io(io::Error),
    Parse(ParseError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "failed to read keyboard shortcuts: {err}"),
            Self::Parse(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Parse(err) => Some(err),
        }
    }
}

fn builtin_chord(action: Action) -> Chord {
    match action {
        Action::Back => Chord {
            ctrl: false,
            alt: true,
            shift: false,
            virtual_key: VK_LEFT,
        },
        Action::Forward => Chord {
            ctrl: false,
            alt: true,
            shift: false,
            virtual_key: VK_RIGHT,
        },
        Action::FocusAddress => Chord {
            ctrl: false,
            alt: true,
            shift: false,
            virtual_key: 0x44, // 'D'
        },
        Action::TogglePane => Chord {
            ctrl: false,
            alt: false,
            shift: false,
            virtual_key: VK_F6,
        },
        Action::NewTab => Chord {
            ctrl: true,
            alt: false,
            shift: false,
            virtual_key: 0x54, // 'T'
        },
        Action::CloseTab => Chord {
            ctrl: true,
            alt: false,
            shift: false,
            virtual_key: 0x57, // 'W'
        },
        Action::NextTab => Chord {
            ctrl: true,
            alt: false,
            shift: false,
            virtual_key: VK_TAB,
        },
        Action::PreviousTab => Chord {
            ctrl: true,
            alt: false,
            shift: true,
            virtual_key: VK_TAB,
        },
        Action::Refresh => Chord {
            ctrl: false,
            alt: false,
            shift: false,
            virtual_key: VK_F5,
        },
    }
}

fn display_key(virtual_key: u16, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match virtual_key {
        VK_LEFT => f.write_str("Left"),
        VK_RIGHT => f.write_str("Right"),
        VK_UP => f.write_str("Up"),
        VK_DOWN => f.write_str("Down"),
        VK_TAB => f.write_str("Tab"),
        VK_F5 => f.write_str("F5"),
        VK_F6 => f.write_str("F6"),
        _ => display_other_key(virtual_key, f),
    }
}

fn display_other_key(virtual_key: u16, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if let Ok(byte) = u8::try_from(virtual_key) {
        if byte.is_ascii_uppercase() {
            return write!(f, "{}", char::from(byte));
        }
    }
    write!(f, "VK{virtual_key:04X}")
}

fn chords_collide(chords: &[Chord]) -> bool {
    chords
        .iter()
        .enumerate()
        .any(|(index, chord)| chords[index + 1..].contains(chord))
}

fn parse_line(line: &str) -> Result<(Action, Chord), ParseError> {
    let Some((action_text, chord_text)) = line.split_once('=') else {
        return Err(ParseError::Invalid);
    };
    let action_text = action_text.trim();
    let chord_text = chord_text.trim();
    if action_text.is_empty() {
        return Err(ParseError::Invalid);
    }
    let action = Action::from_name(action_text).ok_or(ParseError::UnknownAction)?;
    let chord = parse_chord(chord_text)?;
    Ok((action, chord))
}

fn parse_chord(text: &str) -> Result<Chord, ParseError> {
    if text.is_empty() {
        return Err(ParseError::BadChord);
    }
    let parts: Vec<&str> = text.split('+').collect();
    if parts.iter().any(|part| part.is_empty()) {
        return Err(ParseError::BadChord);
    }
    let Some((key, modifiers)) = parts.split_last() else {
        return Err(ParseError::BadChord);
    };
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut last_mod = 0u8;
    for modifier in modifiers {
        let rank = modifier_rank(modifier).ok_or(ParseError::BadChord)?;
        if rank <= last_mod {
            return Err(ParseError::BadChord);
        }
        last_mod = rank;
        match rank {
            1 => ctrl = true,
            2 => alt = true,
            _ => shift = true,
        }
    }
    Ok(Chord {
        ctrl,
        alt,
        shift,
        virtual_key: parse_key(key)?,
    })
}

fn modifier_rank(token: &str) -> Option<u8> {
    if token.eq_ignore_ascii_case("ctrl") {
        Some(1)
    } else if token.eq_ignore_ascii_case("alt") {
        Some(2)
    } else if token.eq_ignore_ascii_case("shift") {
        Some(3)
    } else {
        None
    }
}

fn parse_key(token: &str) -> Result<u16, ParseError> {
    if let Some(letter) = ascii_letter(token) {
        return Ok(letter);
    }
    const NAMED: &[(&str, u16)] = &[
        ("left", VK_LEFT),
        ("right", VK_RIGHT),
        ("up", VK_UP),
        ("down", VK_DOWN),
        ("tab", VK_TAB),
        ("f5", VK_F5),
        ("f6", VK_F6),
    ];
    for (name, virtual_key) in NAMED {
        if token.eq_ignore_ascii_case(name) {
            return Ok(*virtual_key);
        }
    }
    Err(ParseError::BadChord)
}

fn ascii_letter(token: &str) -> Option<u16> {
    let mut bytes = token.bytes();
    let byte = bytes.next()?;
    if bytes.next().is_some() || !byte.is_ascii_alphabetic() {
        return None;
    }
    Some(u16::from(byte.to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "fx-core-shortcuts-{}-{nanos}-{n}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn actions() -> [Action; 9] {
        [
            Action::Back,
            Action::Forward,
            Action::FocusAddress,
            Action::TogglePane,
            Action::NewTab,
            Action::CloseTab,
            Action::NextTab,
            Action::PreviousTab,
            Action::Refresh,
        ]
    }

    fn key(virtual_key: u16) -> Chord {
        Chord {
            ctrl: false,
            alt: false,
            shift: false,
            virtual_key,
        }
    }

    #[test]
    fn builtin_lookup_matches_each_default_and_rejects_a_random_chord() {
        let map = ShortcutMap::builtin();
        let cases = [
            (Action::Back, false, true, false, 0x25, "Alt+Left"),
            (Action::Forward, false, true, false, 0x27, "Alt+Right"),
            (Action::FocusAddress, false, true, false, 0x44, "Alt+D"),
            (Action::TogglePane, false, false, false, 0x75, "F6"),
            (Action::NewTab, true, false, false, 0x54, "Ctrl+T"),
            (Action::CloseTab, true, false, false, 0x57, "Ctrl+W"),
            (Action::NextTab, true, false, false, 0x09, "Ctrl+Tab"),
            (
                Action::PreviousTab,
                true,
                false,
                true,
                0x09,
                "Ctrl+Shift+Tab",
            ),
            (Action::Refresh, false, false, false, 0x74, "F5"),
        ];
        for (action, ctrl, alt, shift, virtual_key, text) in cases {
            let chord = Chord {
                ctrl,
                alt,
                shift,
                virtual_key,
            };
            assert_eq!(map.chord(action), chord);
            assert_eq!(map.lookup(chord), Some(action));
            assert_eq!(chord.to_string(), text);
            assert_eq!(
                ShortcutMap::parse(&format!("{action}={text}")).unwrap(),
                map
            );
        }
        assert_eq!(map.lookup(key(0x42)), None);
        assert_eq!(
            map.lookup(Chord {
                ctrl: false,
                alt: true,
                shift: false,
                virtual_key: 0x26,
            }),
            None
        );
        assert!(map.overrides().is_empty());
    }

    #[test]
    fn builtin_chords_are_unique() {
        let map = ShortcutMap::builtin();
        let actions = actions();
        for (index, action) in actions.iter().enumerate() {
            for other in &actions[index + 1..] {
                assert_ne!(map.chord(*action), map.chord(*other));
            }
        }
    }

    #[test]
    fn rebind_replaces_one_chord_and_chord_in_use_keeps_the_map() {
        let mut map = ShortcutMap::builtin();
        let before = map;
        let taken = map.chord(Action::CloseTab);
        let err = map.rebind(Action::NewTab, taken).unwrap_err();
        assert_eq!(err, RebindError::ChordInUse(Action::CloseTab));
        assert_eq!(map, before);
        assert_eq!(map.lookup(map.chord(Action::NewTab)), Some(Action::NewTab));
        assert_eq!(map.lookup(taken), Some(Action::CloseTab));

        let chord = Chord {
            ctrl: true,
            alt: false,
            shift: false,
            virtual_key: 0x42,
        };
        map.rebind(Action::Back, chord).unwrap();
        assert_eq!(map.chord(Action::Back), chord);
        assert_eq!(map.lookup(chord), Some(Action::Back));
        assert_eq!(
            map.lookup(Chord {
                ctrl: false,
                alt: true,
                shift: false,
                virtual_key: 0x25,
            }),
            None
        );
        let builtin = ShortcutMap::builtin();
        for action in actions() {
            if action != Action::Back {
                assert_eq!(map.chord(action), builtin.chord(action));
            }
        }
        assert_eq!(map.overrides(), vec![(Action::Back, chord)]);
        map.rebind(Action::Back, chord).unwrap();
        assert_eq!(map.overrides(), vec![(Action::Back, chord)]);
    }

    #[test]
    fn parse_one_override_replaces_only_that_action() {
        let map = ShortcutMap::parse("focus_address=Ctrl+L").unwrap();
        let chord = Chord {
            ctrl: true,
            alt: false,
            shift: false,
            virtual_key: 0x4C,
        };
        assert_eq!(map.chord(Action::FocusAddress), chord);
        assert_eq!(map.lookup(chord), Some(Action::FocusAddress));
        assert_eq!(
            map.lookup(Chord {
                ctrl: false,
                alt: true,
                shift: false,
                virtual_key: 0x44,
            }),
            None
        );
        let builtin = ShortcutMap::builtin();
        for action in actions() {
            if action != Action::FocusAddress {
                assert_eq!(map.chord(action), builtin.chord(action));
            }
        }
        assert_eq!(map.overrides(), vec![(Action::FocusAddress, chord)]);
        assert_eq!(ShortcutMap::parse("focus_address=ctrl+l").unwrap(), map);
    }

    #[test]
    fn parse_duplicate_chord_is_an_error() {
        assert_eq!(
            ShortcutMap::parse("back=F5").unwrap_err(),
            ParseError::DuplicateChord
        );
        assert_eq!(
            ShortcutMap::parse("back=Ctrl+K\nclose_tab=Ctrl+K").unwrap_err(),
            ParseError::DuplicateChord
        );
        assert_eq!(
            ShortcutMap::parse("back=Ctrl+K\nback=Ctrl+L").unwrap_err(),
            ParseError::DuplicateAction
        );

        let swapped = ShortcutMap::parse("back=Alt+Right\nforward=Alt+Left").unwrap();
        assert_eq!(swapped.chord(Action::Back).to_string(), "Alt+Right");
        assert_eq!(swapped.chord(Action::Forward).to_string(), "Alt+Left");
        assert_eq!(
            swapped.lookup(Chord {
                ctrl: false,
                alt: true,
                shift: false,
                virtual_key: 0x25,
            }),
            Some(Action::Forward)
        );
    }

    #[test]
    fn parse_unknown_action_is_an_error() {
        assert_eq!(
            ShortcutMap::parse("parent=Alt+Up").unwrap_err(),
            ParseError::UnknownAction
        );
        assert_eq!(
            ShortcutMap::parse("reopen=Ctrl+Shift+T").unwrap_err(),
            ParseError::UnknownAction
        );
        assert_eq!(
            ShortcutMap::parse("Back=Alt+Left").unwrap_err(),
            ParseError::UnknownAction
        );
        assert_eq!(ShortcutMap::parse("back").unwrap_err(), ParseError::Invalid);
        assert_eq!(
            ShortcutMap::parse("back=Alt+F7").unwrap_err(),
            ParseError::BadChord
        );
        assert_eq!(
            ShortcutMap::parse("back=Shift+Ctrl+T").unwrap_err(),
            ParseError::BadChord
        );
        assert_eq!(
            ShortcutMap::parse("back=Ctrl+Ctrl+T").unwrap_err(),
            ParseError::BadChord
        );
    }

    #[test]
    fn parse_empty_is_an_error() {
        assert_eq!(ShortcutMap::parse("").unwrap_err(), ParseError::Empty);
        assert_eq!(
            ShortcutMap::parse(" \r\n\t").unwrap_err(),
            ParseError::Empty
        );
        assert_eq!(ShortcutMap::parse("\n").unwrap_err(), ParseError::Empty);
    }

    #[test]
    fn whitespace_around_a_line_is_accepted() {
        let map = ShortcutMap::parse(" \tnew_tab=Ctrl+N \r\n").unwrap();
        let chord = Chord {
            ctrl: true,
            alt: false,
            shift: false,
            virtual_key: 0x4E,
        };
        assert_eq!(map.chord(Action::NewTab), chord);
        assert_eq!(chord.to_string(), "Ctrl+N");
        let builtin = ShortcutMap::builtin();
        assert_eq!(map.chord(Action::CloseTab), builtin.chord(Action::CloseTab));
        assert_eq!(ShortcutMap::parse("\n  new_tab = ctrl+n \n").unwrap(), map);

        let mixed = ShortcutMap::parse("toggle_pane=ctrl+alt+shift+k").unwrap();
        assert_eq!(
            mixed.chord(Action::TogglePane).to_string(),
            "Ctrl+Alt+Shift+K"
        );
    }

    #[test]
    fn missing_file_returns_builtin_and_does_not_create_it() {
        let dir = TempDir::new();
        let path = dir.0.join("shortcuts.txt");
        assert_eq!(ShortcutMap::load(&path).unwrap(), ShortcutMap::builtin());
        assert!(!path.exists());
    }

    #[test]
    fn empty_file_load_is_a_parse_error() {
        let dir = TempDir::new();
        let path = dir.0.join("shortcuts.txt");
        fs::write(&path, "").unwrap();
        assert!(matches!(
            ShortcutMap::load(&path),
            Err(LoadError::Parse(ParseError::Empty))
        ));

        ShortcutMap::builtin().store(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        assert!(matches!(
            ShortcutMap::load(&path),
            Err(LoadError::Parse(ParseError::Empty))
        ));

        fs::write(&path, "parent=Alt+Up\n").unwrap();
        assert!(matches!(
            ShortcutMap::load(&path),
            Err(LoadError::Parse(ParseError::UnknownAction))
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "parent=Alt+Up\n");

        let nested = dir.0.join("nested");
        fs::create_dir(&nested).unwrap();
        assert!(matches!(ShortcutMap::load(&nested), Err(LoadError::Io(_))));
    }

    #[test]
    fn store_then_load_round_trips_an_override() {
        let dir = TempDir::new();
        let path = dir.0.join("shortcuts.txt");
        let mut map = ShortcutMap::builtin();
        let back = Chord {
            ctrl: true,
            alt: false,
            shift: false,
            virtual_key: 0x42,
        };
        let refresh = Chord {
            ctrl: true,
            alt: false,
            shift: false,
            virtual_key: 0x52,
        };
        map.rebind(Action::Refresh, refresh).unwrap();
        map.rebind(Action::Back, back).unwrap();
        assert_eq!(
            map.overrides(),
            vec![(Action::Back, back), (Action::Refresh, refresh)]
        );
        map.store(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "back=Ctrl+B\nrefresh=Ctrl+R\n"
        );
        let loaded = ShortcutMap::load(&path).unwrap();
        assert_eq!(loaded, map);
        assert_eq!(
            loaded.chord(Action::Forward),
            ShortcutMap::builtin().chord(Action::Forward)
        );
    }
}
