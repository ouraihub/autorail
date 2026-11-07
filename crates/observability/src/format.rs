//! Stable, grep-friendly text output for the native diagnostic log.

use std::fmt;

use tracing::{Event, Subscriber, field::Visit};
use tracing_subscriber::{
    fmt::{
        FmtContext, FormattedFields,
        format::{FormatEvent, FormatFields, Writer},
    },
    registry::LookupSpan,
};

pub(crate) struct FlatFormat<T> {
    pub(crate) timer: T,
}

struct EventFields {
    fields: Vec<(String, String)>,
    message: Option<String>,
}

impl EventFields {
    fn new() -> Self {
        Self {
            fields: Vec::new(),
            message: None,
        }
    }
}

impl Visit for EventFields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        let rendered = format!("{value:?}");
        if field.name() == "message" {
            self.message = Some(rendered.trim_matches('"').to_owned());
        } else {
            self.fields.push((field.name().to_owned(), rendered));
        }
    }
}

impl<S, N, T> FormatEvent<S, N> for FlatFormat<T>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    T: tracing_subscriber::fmt::time::FormatTime,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        self.timer.format_time(&mut writer)?;
        write!(writer, " {}", event.metadata().level())?;

        let mut event_fields = EventFields::new();
        event.record(&mut event_fields);
        let mut trace_id = None;
        let mut operation = None;
        let mut component = None;
        let mut chat_id = None;
        let mut workspace_id = None;
        let mut backend = None;
        let mut span_id = None;
        let mut parent_span_id = None;

        if let Some(scope) = ctx.event_scope() {
            let spans: Vec<_> = scope.from_root().collect();
            if let Some(current) = spans.last() {
                span_id = Some(current.id().into_u64());
            }
            if spans.len() > 1 {
                parent_span_id = Some(spans[spans.len() - 2].id().into_u64());
            }
            for span in spans {
                let extensions = span.extensions();
                let Some(span_fields) = extensions.get::<FormattedFields<N>>() else {
                    continue;
                };
                trace_id = field_value(&span_fields.fields, "trace_id").or(trace_id);
                operation = field_value(&span_fields.fields, "operation").or(operation);
                component = field_value(&span_fields.fields, "component")
                    .or_else(|| field_value(&span_fields.fields, "layer"))
                    .or(component);
                chat_id = field_value(&span_fields.fields, "chat_id").or(chat_id);
                workspace_id = field_value(&span_fields.fields, "workspace_id").or(workspace_id);
                backend = field_value(&span_fields.fields, "backend").or(backend);
            }
        }
        component.get_or_insert_with(|| event.metadata().target().to_owned());
        write_optional(&mut writer, "trace_id", trace_id.as_deref())?;
        write_number(&mut writer, "span_id", span_id)?;
        write_number(&mut writer, "parent_span_id", parent_span_id)?;
        write_optional(&mut writer, "operation", operation.as_deref())?;
        write_optional(&mut writer, "component", component.as_deref())?;
        write_optional(&mut writer, "chat_id", chat_id.as_deref())?;
        write_optional(&mut writer, "workspace_id", workspace_id.as_deref())?;
        write_optional(&mut writer, "backend", backend.as_deref())?;
        for (key, value) in event_fields.fields {
            write!(writer, " {key}={value}")?;
        }
        if let Some(message) = event_fields.message {
            write!(writer, " message=\"{message}\"")?;
        }
        writeln!(writer)
    }
}

fn write_optional(writer: &mut Writer<'_>, key: &str, value: Option<&str>) -> fmt::Result {
    if let Some(value) = value {
        write!(writer, " {key}={value}")?;
    }
    Ok(())
}

fn write_number(writer: &mut Writer<'_>, key: &str, value: Option<u64>) -> fmt::Result {
    if let Some(value) = value {
        write!(writer, " {key}={value}")?;
    }
    Ok(())
}

fn field_value(input: &str, wanted: &str) -> Option<String> {
    parse_fields(input)
        .into_iter()
        .rev()
        .find_map(|(key, value)| (key == wanted).then_some(value.to_owned()))
}

fn parse_fields(input: &str) -> Vec<(&str, &str)> {
    let mut result = Vec::new();
    let bytes = input.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let key_start = cursor;
        while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() && bytes[cursor] != b'='
        {
            cursor += 1;
        }
        if cursor == key_start || bytes.get(cursor) != Some(&b'=') {
            break;
        }
        let key = &input[key_start..cursor];
        cursor += 1;
        let value_start = cursor;
        if bytes.get(cursor) == Some(&b'"') {
            cursor += 1;
            let mut escaped = false;
            while cursor < bytes.len() {
                if escaped {
                    escaped = false;
                } else if bytes[cursor] == b'\\' {
                    escaped = true;
                } else if bytes[cursor] == b'"' {
                    cursor += 1;
                    break;
                }
                cursor += 1;
            }
        } else {
            while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
        }
        result.push((key, &input[value_start..cursor]));
    }
    result
}
