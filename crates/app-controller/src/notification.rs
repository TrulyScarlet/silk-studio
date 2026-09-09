//! Notification dispatch interface for save outcomes (NOT-001..002 in S6;
//! open/reveal actions are owned by the later library surface). The controller
//! routes every user-facing save outcome through here so UI wiring cannot
//! bypass it.

use std::sync::Mutex;

/// Coarse severity mirrored into toast/banner styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationSeverity {
    Info,
    Warning,
    Error,
}

impl NotificationSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationContent {
    pub severity: NotificationSeverity,
    pub title: String,
    pub body: String,
}

impl NotificationContent {
    pub fn info(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            severity: NotificationSeverity::Info,
            title: title.into(),
            body: body.into(),
        }
    }

    pub fn error(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            severity: NotificationSeverity::Error,
            title: title.into(),
            body: body.into(),
        }
    }
}

/// Sink implemented by the UI layer; test doubles record instead.
pub trait NotificationSink: Send + Sync {
    fn notify(&self, notification: &NotificationContent);
}

/// Logs through the diagnostics pipeline; useful headless and in tests.
pub struct ConsoleNotificationSink;

impl NotificationSink for ConsoleNotificationSink {
    fn notify(&self, notification: &NotificationContent) {
        diagnostics::info(
            "notifications",
            &format!(
                "[{}] {}: {}",
                notification.severity.as_str(),
                notification.title,
                notification.body
            ),
        );
    }
}

/// Records notifications for assertions.
#[derive(Default)]
pub struct CollectingNotificationSink(Mutex<Vec<NotificationContent>>);

impl CollectingNotificationSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Vec<NotificationContent> {
        self.0.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl NotificationSink for CollectingNotificationSink {
    fn notify(&self, notification: &NotificationContent) {
        if let Ok(mut guard) = self.0.lock() {
            guard.push(notification.clone());
        }
    }
}
