use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// A fresh random identifier.
            pub fn generate() -> Self { Self(uuid::Uuid::new_v4().to_string()) }
            pub fn as_str(&self) -> &str { &self.0 }
        }

        impl From<String> for $name { fn from(value: String) -> Self { Self(value) } }
        impl From<&str> for $name { fn from(value: &str) -> Self { Self(value.to_owned()) } }
        impl AsRef<str> for $name { fn as_ref(&self) -> &str { &self.0 } }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
        }
    };
}

id!(ProjectId);
id!(WorkspaceId);
id!(
    /// Host-owned chat identity. Engine-native locators are a separate [`NativeSessionKey`].
    ChatId
);
id!(
    /// Engine-native resume locator (pi/OMP session file, DSH session id). Opaque to the host.
    NativeSessionKey
);
id!(TerminalRuntimeId);
id!(ReviewId);
id!(PlanItemId);
id!(ProviderId);
id!(ProviderKeyId);
id!(McpServerId);
