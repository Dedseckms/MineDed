//! Turns the log4j2 `XMLLayout` console output (enabled by Mojang's
//! `client-1.12.xml` logging config) into plain text lines like the vanilla
//! console: `[12:34:56] [Render thread/INFO]: message`.
//!
//! Input arrives line by line; an event may span many lines (multi-line
//! messages, stack traces), so the parser buffers until `</log4j:Event>`.
//! Non-XML lines (JVM output before log4j starts, stderr) pass through as is.

const EVENT_OPEN: &str = "<log4j:Event";
const EVENT_CLOSE: &str = "</log4j:Event>";
/// A broken/unterminated event is flushed raw once it grows past this.
const MAX_EVENT: usize = 1024 * 1024;

#[derive(Default)]
pub(crate) struct Log4jParser {
    /// Text of the current unterminated event, `None` outside an event.
    pending: Option<String>,
}

impl Log4jParser {
    /// Feeds one line (without the trailing newline); returns finished output lines.
    pub fn feed(&mut self, line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = line.to_owned();
        loop {
            match self.pending.as_mut() {
                None => match rest.find(EVENT_OPEN) {
                    None => {
                        if !rest.trim().is_empty() || (out.is_empty() && rest == line) {
                            out.push(rest);
                        }
                        return out;
                    }
                    Some(i) => {
                        let before = &rest[..i];
                        if !before.trim().is_empty() {
                            out.push(before.trim_end().to_owned());
                        }
                        self.pending = Some(rest[i..].to_owned());
                        rest = String::new();
                    }
                },
                Some(buf) => {
                    if !rest.is_empty() {
                        buf.push('\n');
                        buf.push_str(&rest);
                    }
                    match buf.find(EVENT_CLOSE) {
                        Some(end) => {
                            let tail = buf.split_off(end + EVENT_CLOSE.len());
                            let event = std::mem::take(buf);
                            self.pending = None;
                            render_event(&event, &mut out);
                            if tail.trim().is_empty() {
                                return out;
                            }
                            rest = tail;
                        }
                        None => {
                            if buf.len() > MAX_EVENT {
                                out.extend(self.finish());
                            }
                            return out;
                        }
                    }
                }
            }
        }
    }

    /// Flushes an unterminated event as raw lines (stream ended mid-event).
    pub fn finish(&mut self) -> Vec<String> {
        match self.pending.take() {
            Some(buf) => buf.lines().map(str::to_owned).collect(),
            None => Vec::new(),
        }
    }
}

fn render_event(event: &str, out: &mut Vec<String>) {
    let open_end = event.find('>').unwrap_or(event.len());
    let tag = &event[..open_end];
    let level = attr(tag, "level").unwrap_or_default();
    let thread = attr(tag, "thread").unwrap_or_default();
    let time = attr(tag, "timestamp")
        .and_then(|t| t.parse::<i64>().ok())
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%H:%M:%S")
                .to_string()
        });
    let body = event.get(open_end + 1..).unwrap_or("");
    let message = element(body, "log4j:Message").unwrap_or_default();
    let throwable = element(body, "log4j:Throwable");

    let prefix = match time {
        Some(t) => format!("[{t}] [{thread}/{level}]: "),
        None => format!("[{thread}/{level}]: "),
    };
    let mut lines = message.trim_end_matches(['\r', '\n']).lines();
    out.push(format!("{prefix}{}", lines.next().unwrap_or("")));
    out.extend(lines.map(str::to_owned));
    if let Some(t) = throwable {
        out.extend(t.trim_end_matches(['\r', '\n']).lines().map(str::to_owned));
    }
}

/// `name="value"` from an opening tag, XML-unescaped.
fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let len = tag[start..].find('"')?;
    Some(unescape(&tag[start..start + len]))
}

/// Text content of `<name>...</name>`: CDATA sections verbatim, the rest unescaped.
fn element(body: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = body.find(&open)? + open.len();
    let len = body[start..].find(&close)?;
    let mut raw = &body[start..start + len];
    let mut text = String::new();
    while !raw.is_empty() {
        match raw.find("<![CDATA[") {
            Some(i) => {
                text.push_str(&unescape(&raw[..i]));
                let after = &raw[i + 9..];
                let end = after.find("]]>").unwrap_or(after.len());
                text.push_str(&after[..end]);
                raw = after.get(end + 3..).unwrap_or("");
            }
            None => {
                text.push_str(&unescape(raw));
                break;
            }
        }
    }
    Some(text)
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#13;", "\r")
        .replace("&#10;", "\n")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &str) -> Vec<String> {
        let mut p = Log4jParser::default();
        let mut out: Vec<String> = input.lines().flat_map(|l| p.feed(l)).collect();
        out.extend(p.finish());
        out
    }

    /// Strips the local-time `[HH:MM:SS] ` prefix (timezone dependent).
    fn untimed(lines: Vec<String>) -> Vec<String> {
        lines
            .into_iter()
            .map(|l| match l.strip_prefix('[') {
                Some(r) if r.len() > 10 && r.as_bytes()[8] == b']' => r[10..].to_owned(),
                _ => l,
            })
            .collect()
    }

    // Real 1.21.1 console output with client-1.12.xml.
    const SAMPLE: &str = r#"<log4j:Event logger="net.minecraft.client.Minecraft" timestamp="1729700000000" level="INFO" thread="Render thread">
  <log4j:Message><![CDATA[Setting user: Player]]></log4j:Message>
</log4j:Event>
<log4j:Event logger="com.mojang.blaze3d.platform.GLX" timestamp="1729700000100" level="WARN" thread="Render thread">
  <log4j:Message><![CDATA[Failed to load <x> & "y"]]></log4j:Message>
</log4j:Event>
<log4j:Event logger="net.minecraft.server.MinecraftServer" timestamp="1729700000200" level="ERROR" thread="Server thread">
  <log4j:Message><![CDATA[Encountered an unexpected exception]]></log4j:Message>
  <log4j:Throwable><![CDATA[java.lang.IllegalStateException: boom
	at net.minecraft.server.MinecraftServer.w(MinecraftServer.java:123)
	at java.base/java.lang.Thread.run(Thread.java:1583)
]]></log4j:Throwable>
</log4j:Event>
"#;

    #[test]
    fn parses_events() {
        let out = untimed(run(SAMPLE));
        assert_eq!(
            out,
            vec![
                "[Render thread/INFO]: Setting user: Player",
                "[Render thread/WARN]: Failed to load <x> & \"y\"",
                "[Server thread/ERROR]: Encountered an unexpected exception",
                "java.lang.IllegalStateException: boom",
                "\tat net.minecraft.server.MinecraftServer.w(MinecraftServer.java:123)",
                "\tat java.base/java.lang.Thread.run(Thread.java:1583)",
            ]
        );
    }

    #[test]
    fn timestamp_prefix_present() {
        let out = run(SAMPLE);
        let first = &out[0];
        assert!(first.starts_with('['), "{first}");
        assert_eq!(first.as_bytes()[9], b']', "{first}");
    }

    #[test]
    fn plain_lines_pass_through_and_mix() {
        let input = "Picked up JAVA_TOOL_OPTIONS: -Dfoo\n\
            <log4j:Event logger=\"a\" timestamp=\"1\" level=\"DEBUG\" thread=\"main\"><log4j:Message><![CDATA[one]]></log4j:Message></log4j:Event><log4j:Event logger=\"a\" timestamp=\"1\" level=\"INFO\" thread=\"main\"><log4j:Message>a &lt;b&gt;</log4j:Message></log4j:Event>\n\
            trailing plain";
        let out = untimed(run(input));
        assert_eq!(
            out,
            vec![
                "Picked up JAVA_TOOL_OPTIONS: -Dfoo",
                "[main/DEBUG]: one",
                "[main/INFO]: a <b>",
                "trailing plain",
            ]
        );
    }

    #[test]
    fn multiline_message_and_split_cdata() {
        let input = "<log4j:Event logger=\"a\" timestamp=\"1\" level=\"INFO\" thread=\"t\">\n\
            <log4j:Message><![CDATA[line1\nline2 ]]]]><![CDATA[> end]]></log4j:Message>\n\
            </log4j:Event>";
        assert_eq!(
            untimed(run(input)),
            vec!["[t/INFO]: line1", "line2 ]]> end"]
        );
    }

    #[test]
    fn unterminated_event_flushed_raw() {
        let mut p = Log4jParser::default();
        assert!(p
            .feed("<log4j:Event level=\"INFO\" thread=\"t\">")
            .is_empty());
        assert!(p.feed("  <log4j:Message><![CDATA[x").is_empty());
        assert_eq!(p.finish().len(), 2);
        assert_eq!(p.feed("plain"), vec!["plain"]);
    }

    #[test]
    fn empty_line_outside_event_kept() {
        let mut p = Log4jParser::default();
        assert_eq!(p.feed(""), vec![""]);
    }
}
