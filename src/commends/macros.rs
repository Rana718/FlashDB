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

            pub fn from_str(value: &str) -> Option<Self> {
                match value.trim().to_ascii_uppercase().as_str() {
                    $($value => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::from_str(&value).unwrap_or(Self::$default)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::from_str(value).unwrap_or(Self::$default)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}
