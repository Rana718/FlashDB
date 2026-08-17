#[macro_export]
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            default = $default:ident;

            $($variant:ident => $value:literal),* $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $($variant),*
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),*
                }
            }

            pub fn from_bytes(value: &[u8]) -> Self {
                use std::sync::OnceLock;
                use foldhash::{HashMap, HashMapExt};

                static MAP: OnceLock<HashMap<&'static str, $name>> = OnceLock::new();
                let map = MAP.get_or_init(|| {
                    let mut m = HashMap::new();
                    $(m.insert($value, $name::$variant);)*
                    m
                });

                if value.len() > 32 {
                    return Self::$default;
                }
                let mut buf = [0u8; 32];
                let len = value.len();
                for (i, &b) in value.iter().enumerate() {
                    buf[i] = b.to_ascii_uppercase();
                }
                let upper = unsafe { std::str::from_utf8_unchecked(&buf[..len]) };
                map.get(upper).copied().unwrap_or(Self::$default)
            }
        }

        impl From<String> for $name {
            #[inline(always)]
            fn from(value: String) -> Self {
                Self::from_bytes(value.as_bytes())
            }
        }

        impl From<&str> for $name {
            #[inline(always)]
            fn from(value: &str) -> Self {
                Self::from_bytes(value.as_bytes())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}
