//! Atoll's own Google OAuth client ("Desktop app" type), so Connect works
//! without any setup. Google treats installed-app secrets as not confidential:
//! the security comes from PKCE and the loopback redirect, not this value.
//! A client ID and secret saved in Settings (Advanced) take precedence.

pub const CLIENT_ID: &str = "647360626322-p1n7vmqjavqf7bsvc9isqfmd8bceskej.apps.googleusercontent.com";
pub const CLIENT_SECRET: &str = "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";