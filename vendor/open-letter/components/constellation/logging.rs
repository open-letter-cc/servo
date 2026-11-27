// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/constellation/logging.rs`.

#[allow(unused_imports)]
use super::*;

/// Run `log` unless this thread is already inside one of the loggers here.
///
/// Sending a record can itself log: a failed IPC send logs its I/O error. That record
/// arrives back in the logger on the same thread, under a re-entrant lock, and sending
/// it can fail the same way, so without this the thread recurses until its stack
/// overflows (as when the pipe to the constellation has become an invalid handle). A
/// record logged while sending another is therefore dropped.
pub(super) fn without_reentry(log: impl FnOnce()) {
    if IN_LOGGER.replace(true) {
        return;
    }
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            IN_LOGGER.set(false);
        }
    }
    let _leave = Leave;
    log();
}

impl FromScriptLogger {
    pub(super) fn send(&self, record: &Record) {
        if let Some(entry) = log_entry(record) {
            let thread_name = thread::current().name().map(ToOwned::to_owned);
            let _ = self.script_to_constellation_sender.lock().send(
                // TODO: The WebViewId and PipelineId are unused for the `LogEntry` method, but it
                // would probably be better to have an entirely new message variant or sender for
                // messages that definitely do not need these fields to avoid having to use the
                // TEST_* values here.
                (
                    TEST_WEBVIEW_ID,
                    TEST_PIPELINE_ID,
                    ScriptToConstellationMessage::LogEntry(
                        ScriptEventLoopId::installed(),
                        thread_name,
                        entry,
                    ),
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use log::{Level, Log, Record};
    use servo_base::generic_channel;

    use super::{FromEmbedderLogger, FromScriptLogger, without_reentry};

    fn log_a_warning(logger: &dyn Log) {
        logger.log(
            &Record::builder()
                .level(Level::Warn)
                .args(format_args!("a warning"))
                .build(),
        );
    }

    /// A record logged while a logger here is already logging on the same thread (as a
    /// failed IPC send logs its error) is dropped, not sent: sending it could fail again
    /// and recurse until the stack overflows.
    #[test]
    fn a_record_logged_while_logging_is_dropped() {
        let (embedder_sender, embedder_receiver) = crossbeam_channel::unbounded();
        let embedder_logger = FromEmbedderLogger::new(embedder_sender);
        let (script_sender, script_receiver) =
            generic_channel::channel().expect("Should be able to create a channel");
        let script_logger = FromScriptLogger::new(script_sender);

        for logger in [&embedder_logger as &dyn Log, &script_logger as &dyn Log] {
            without_reentry(|| log_a_warning(logger));
        }
        // Nested, as when sending a record logs: the inner record goes nowhere.
        without_reentry(|| {
            log_a_warning(&embedder_logger);
            log_a_warning(&script_logger);
        });
        assert!(embedder_receiver.try_recv().is_err());
        assert!(script_receiver.try_recv().is_err());

        // Outside a logger, records are sent.
        log_a_warning(&embedder_logger);
        log_a_warning(&script_logger);
        assert!(embedder_receiver.try_recv().is_ok());
        assert!(script_receiver.try_recv().is_ok());
    }

    /// Being inside a logger ends with the logging, also when the logging panics, so
    /// later records on the thread are still sent.
    #[test]
    fn logging_resumes_after_a_panic_while_logging() {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let logger = FromEmbedderLogger::new(sender);
        let result = catch_unwind(AssertUnwindSafe(|| {
            without_reentry(|| panic!("while logging"))
        }));
        assert!(result.is_err());
        log_a_warning(&logger);
        assert!(receiver.try_recv().is_ok());
    }
}
