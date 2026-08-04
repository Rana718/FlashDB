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

            #[inline(always)]
            pub fn from_bytes(value: &[u8]) -> Self {
                $(
                    if value.len() == $value.len() && value.eq_ignore_ascii_case($value.as_bytes()) {
                        return Self::$variant;
                    }
                )*
                Self::$default
            }
        }

        impl From<String> for $name {
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
