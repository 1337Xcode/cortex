//! Enums whose values each have one fixed name, used both in storage and on the wire.

/// Declares an enum whose variants each have one name, with `ALL`, `NAMES`, `name` and
/// `from_name`, serialized as its name. The names are written to the database and sent between
/// processes, so each one is fixed once chosen.
macro_rules! named_kind {
    (
        $(#[$meta:meta])*
        $kind:ident { $( $(#[$variant_meta:meta])* $variant:ident => $name:literal ),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $kind {
            $( $(#[$variant_meta])* $variant, )+
        }

        impl $kind {
            /// Every value, in declaration order.
            pub const ALL: &'static [Self] = &[ $( Self::$variant, )+ ];

            /// Every name, in declaration order.
            pub const NAMES: &'static [&'static str] = &[ $( $name, )+ ];

            /// The name.
            #[must_use]
            pub fn name(self) -> &'static str {
                match self {
                    $( Self::$variant => $name, )+
                }
            }

            /// The value with this name.
            #[must_use]
            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|value| value.name() == name)
            }
        }

        impl serde::Serialize for $kind {
            /// The name, as a string.
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.name())
            }
        }

        impl<'de> serde::Deserialize<'de> for $kind {
            /// The value whose name the string is; any other string is an unknown variant.
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let name = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
                Self::from_name(&name)
                    .ok_or_else(|| serde::de::Error::unknown_variant(&name, Self::NAMES))
            }
        }
    };
}

pub(crate) use named_kind;
