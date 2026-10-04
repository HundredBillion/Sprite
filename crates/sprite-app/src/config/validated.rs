use std::{collections::BTreeMap, fmt, ops::Deref};

macro_rules! metric {
    ($name:ident, $min:expr, $max:expr, $default:expr) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct $name(f32);
        impl $name {
            pub fn new(value: f32) -> Self {
                Self(if value.is_nan() {
                    $default
                } else {
                    value.clamp($min, $max)
                })
            }
            pub fn get(self) -> f32 {
                self.0
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new($default)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl PartialEq<f32> for $name {
            fn eq(&self, other: &f32) -> bool {
                self.0 == *other
            }
        }
    };
}
metric!(
    FontSize,
    super::Font::MIN_SIZE,
    super::Font::MAX_SIZE,
    super::Font::DEFAULT_SIZE
);
metric!(
    LineHeight,
    super::Font::MIN_LINE_HEIGHT,
    super::Font::MAX_LINE_HEIGHT,
    super::Font::DEFAULT_LINE_HEIGHT
);
metric!(
    Padding,
    0.0,
    super::Grid::MAX_PADDING,
    super::Grid::DEFAULT_PADDING
);

macro_rules! bytes {
    ($name:ident, $repr:ty, $max:expr) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct $name($repr);
        impl $name {
            pub fn new(value: $repr) -> Self {
                Self(value.min($max))
            }
            pub fn get(self) -> $repr {
                self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl PartialEq<$repr> for $name {
            fn eq(&self, other: &$repr) -> bool {
                self.0 == *other
            }
        }
    };
}
bytes!(ScrollbackBytes, usize, super::Scrollback::MAX_BYTES);
bytes!(StorageBytes, u64, i64::MAX as u64);
bytes!(
    TextureBytes,
    usize,
    usize::try_from(i64::MAX).unwrap_or(usize::MAX)
);

/// UTF-8 preferences trim surrounding whitespace; blank-only values mean no preference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NonBlank(String);
impl NonBlank {
    pub fn new(value: String) -> Option<Self> {
        let value = value.trim();
        (!value.is_empty()).then(|| Self(value.to_owned()))
    }
}
impl Deref for NonBlank {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

/// Sorted unique keys; the last supplied value wins, including numeric palette aliases.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalMap<K, V>(Vec<(K, V)>);
impl<K, V> Default for CanonicalMap<K, V> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
impl<K: Ord, V> From<Vec<(K, V)>> for CanonicalMap<K, V> {
    fn from(entries: Vec<(K, V)>) -> Self {
        Self(
            entries
                .into_iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        )
    }
}
impl<K, V> Deref for CanonicalMap<K, V> {
    type Target = [(K, V)];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<'a, K, V> IntoIterator for &'a CanonicalMap<K, V> {
    type Item = &'a (K, V);
    type IntoIter = std::slice::Iter<'a, (K, V)>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
impl<K: PartialEq, V: PartialEq> PartialEq<Vec<(K, V)>> for CanonicalMap<K, V> {
    fn eq(&self, other: &Vec<(K, V)>) -> bool {
        self.0 == *other
    }
}

/// Files contain UTF-8; CLI commands keep their independent OsString representation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Shell {
    pub program: Option<NonBlank>,
    pub args: Option<Vec<String>>,
    pub startup_directory: Option<NonBlank>,
}
impl Shell {
    pub fn session_preference(&self) -> sprite_term::ShellPreference {
        sprite_term::ShellPreference {
            program: self.program.as_deref().map(std::path::PathBuf::from),
            args: self
                .args
                .as_ref()
                .map(|args| args.iter().map(std::ffi::OsString::from).collect()),
            startup_directory: self
                .startup_directory
                .as_deref()
                .map(std::path::PathBuf::from),
        }
    }
}
