//! Replay of recorded NMEA 0183 / AIS logs.
//!
//! Timing comes from, in order of preference:
//! 1. NMEA 4 TAG blocks with a source time (`\c:<unix s or ms>*hh\`), as written by the file
//!    output;
//! 2. times inside the sentences (RMC, GGA, GLL, ZDA, GNS, GBS, BWC…), with midnight roll-over;
//! 3. a fixed rate of 10 lines per second when the log has no times at all.
//!
//! A log typically has bursts: several lines sharing one sentence time. Lines whose time
//! strictly increases are anchors; the lines between two anchors are spread evenly between
//! them, which reproduces the original rhythm without bunching each second's traffic.

use protocols::nmea0183::sentence_id;
use sim_core::LatLon;

/// Lines per second used for logs without any time information.
pub const UNTIMED_RATE: f64 = 10.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Seconds from the start of the log.
    pub t: f64,
    /// The sentence without TAG block and line ending.
    pub line: String,
    /// Position carried by the sentence (RMC, GGA, GLL), for display.
    pub fix: Option<LatLon>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplayLog {
    pub name: String,
    pub entries: Vec<Entry>,
    /// Absolute UTC time of the first entry when known (TAG blocks or RMC/ZDA date).
    pub start_unix: Option<f64>,
    /// Where the timing came from.
    pub timing: Timing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timing {
    TagBlocks,
    SentenceTimes,
    FixedRate,
}

impl Timing {
    pub fn label(self) -> &'static str {
        match self {
            Timing::TagBlocks => "TAG block times",
            Timing::SentenceTimes => "sentence times",
            Timing::FixedRate => "no times: 10 lines/s",
        }
    }
}

impl ReplayLog {
    pub fn duration(&self) -> f64 {
        self.entries.last().map_or(0.0, |e| e.t)
    }

    pub fn parse(name: &str, text: &str) -> Result<ReplayLog, String> {
        struct Raw {
            line: String,
            tag_time: Option<f64>,
            /// Seconds since midnight, and the date's Unix midnight when the sentence has one.
            tod: Option<f64>,
            date: Option<f64>,
            fix: Option<LatLon>,
        }
        let mut raws = Vec::new();
        for l in text.lines() {
            let l = l.trim();
            let (tag, body) = split_tag_block(l);
            if !(body.starts_with('$') || body.starts_with('!')) {
                continue;
            }
            let (tod, date, fix) = sentence_time_and_fix(body);
            raws.push(Raw { line: body.to_string(), tag_time: tag, tod, date, fix });
        }
        if raws.is_empty() {
            return Err("no NMEA sentences found".into());
        }

        // Absolute or relative time per line (None = untimed).
        let (timing, times): (Timing, Vec<Option<f64>>) = if raws.iter().any(|r| r.tag_time.is_some()) {
            (Timing::TagBlocks, raws.iter().map(|r| r.tag_time).collect())
        } else if raws.iter().any(|r| r.tod.is_some()) {
            // Unwrap time of day across midnight.
            let mut day = 0.0;
            let mut last: Option<f64> = None;
            let mut v = Vec::with_capacity(raws.len());
            for r in &raws {
                v.push(r.tod.map(|t| {
                    if let Some(prev) = last
                        && t + day < prev - 43_200.0
                    {
                        day += 86_400.0;
                    }
                    let abs = t + day;
                    last = Some(abs);
                    abs
                }));
            }
            (Timing::SentenceTimes, v)
        } else {
            (Timing::FixedRate, raws.iter().enumerate().map(|(i, _)| Some(i as f64 / UNTIMED_RATE)).collect())
        };

        // Anchors: lines whose time strictly increases.
        let mut anchors: Vec<(usize, f64)> = Vec::new();
        for (i, t) in times.iter().enumerate() {
            if let Some(t) = *t
                && anchors.last().is_none_or(|(_, last)| t > *last)
            {
                anchors.push((i, t));
            }
        }
        let t0 = anchors.first().map(|a| a.1).unwrap_or(0.0);
        let mut rel = vec![0.0; raws.len()];
        for w in anchors.windows(2) {
            let ((i, ti), (j, tj)) = (w[0], w[1]);
            for (k, slot) in rel.iter_mut().enumerate().take(j).skip(i) {
                *slot = ti - t0 + (tj - ti) * (k - i) as f64 / (j - i) as f64;
            }
        }
        if let Some(&(last_i, last_t)) = anchors.last() {
            // Lines after the last anchor follow at the average rate seen so far, at most 1 s apart.
            let n = raws.len() - last_i;
            let span = last_t - t0;
            let step = if last_i > 0 && span > 0.0 { (span / last_i as f64).min(1.0) } else { 1.0 / UNTIMED_RATE };
            for (k, slot) in rel.iter_mut().enumerate().skip(last_i).take(n) {
                *slot = last_t - t0 + step * (k - last_i) as f64;
            }
        }

        let start_unix = match timing {
            Timing::TagBlocks => Some(t0),
            Timing::SentenceTimes => raws.iter().find_map(|r| Some(r.date? + r.tod?)).map(|first_dated| {
                // Shift the dated line's time back to the log start.
                let idx = raws.iter().position(|r| r.date.is_some() && r.tod.is_some()).unwrap_or(0);
                first_dated - rel[idx]
            }),
            Timing::FixedRate => None,
        };
        let entries = raws.into_iter().zip(rel).map(|(r, t)| Entry { t, line: r.line, fix: r.fix }).collect();
        Ok(ReplayLog { name: name.to_string(), entries, start_unix, timing })
    }
}

/// Splits a leading TAG block; returns its source time (`c:`) in Unix seconds.
fn split_tag_block(l: &str) -> (Option<f64>, &str) {
    let Some(rest) = l.strip_prefix('\\') else { return (None, l) };
    let Some(end) = rest.find('\\') else { return (None, l) };
    let block = &rest[..end];
    let body = &rest[end + 1..];
    let fields = block.split('*').next().unwrap_or("");
    let time = fields.split(',').find_map(|f| f.strip_prefix("c:")).and_then(|v| v.parse::<f64>().ok()).map(|v| {
        // Seconds (10 digits today) or milliseconds (13 digits).
        if v > 1e11 { v / 1000.0 } else { v }
    });
    (time, body)
}

fn hhmmss(f: &str) -> Option<f64> {
    if f.len() < 6 {
        return None;
    }
    let h: f64 = f.get(0..2)?.parse().ok()?;
    let m: f64 = f.get(2..4)?.parse().ok()?;
    let s: f64 = f.get(4..)?.parse().ok()?;
    (h < 24.0 && m < 60.0 && s < 61.0).then_some(h * 3600.0 + m * 60.0 + s)
}

fn ddmmyy(f: &str) -> Option<f64> {
    if f.len() != 6 {
        return None;
    }
    let d: u32 = f.get(0..2)?.parse().ok()?;
    let m: u32 = f.get(2..4)?.parse().ok()?;
    let y: i32 = f.get(4..6)?.parse().ok()?;
    let y = if y < 80 { 2000 + y } else { 1900 + y };
    unix_midnight(y, m, d)
}

fn unix_midnight(y: i32, m: u32, d: u32) -> Option<f64> {
    chrono::NaiveDate::from_ymd_opt(y, m, d)
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|dt| dt.and_utc().timestamp() as f64)
}

fn coord(v: &str, hemi: &str, deg_digits: usize) -> Option<f64> {
    if v.len() < deg_digits + 2 {
        return None;
    }
    let deg: f64 = v.get(..deg_digits)?.parse().ok()?;
    let min: f64 = v.get(deg_digits..)?.parse().ok()?;
    let x = deg + min / 60.0;
    match hemi {
        "N" | "E" => Some(x),
        "S" | "W" => Some(-x),
        _ => None,
    }
}

fn latlon(f: &[&str], i: usize) -> Option<LatLon> {
    Some(LatLon::new(coord(f.get(i)?, f.get(i + 1)?, 2)?, coord(f.get(i + 2)?, f.get(i + 3)?, 3)?))
}

/// Time of day (s), date (Unix midnight) and position carried by a sentence.
fn sentence_time_and_fix(line: &str) -> (Option<f64>, Option<f64>, Option<LatLon>) {
    let body = line.split('*').next().unwrap_or(line);
    let f: Vec<&str> = body.split(',').collect();
    let get = |i: usize| f.get(i).copied().unwrap_or("");
    match sentence_id(line).unwrap_or("") {
        "RMC" => {
            let fix = if get(2) == "A" { latlon(&f, 3) } else { None };
            (hhmmss(get(1)), ddmmyy(get(9)), fix)
        }
        "GGA" => {
            let fix = if get(6) != "0" && !get(6).is_empty() { latlon(&f, 2) } else { None };
            (hhmmss(get(1)), None, fix)
        }
        "GLL" => (hhmmss(get(5)), None, if get(6) == "A" { latlon(&f, 1) } else { None }),
        "ZDA" => {
            let date = (|| unix_midnight(get(4).parse().ok()?, get(3).parse().ok()?, get(2).parse().ok()?))();
            (hhmmss(get(1)), date, None)
        }
        "GNS" | "GBS" | "GST" | "BWC" | "BWR" | "TLL" | "TTM" => (hhmmss(get(1)), None, None),
        _ => (None, None, None),
    }
}

/// Playback state of a loaded log.
#[derive(Debug, Clone)]
pub struct Player {
    pub log: std::sync::Arc<ReplayLog>,
    /// Next entry to send.
    pub cursor: usize,
    /// Playback position, seconds from the start of the log.
    pub clock: f64,
    pub playing: bool,
    pub speed: f64,
    pub looped: bool,
    /// Do not send the simulation's own data while a log is loaded.
    pub mute_simulation: bool,
    pub last_fix: Option<LatLon>,
    pub sent: u64,
}

impl Player {
    pub fn new(log: ReplayLog) -> Self {
        Self {
            log: std::sync::Arc::new(log),
            cursor: 0,
            clock: 0.0,
            playing: true,
            speed: 1.0,
            looped: false,
            mute_simulation: true,
            last_fix: None,
            sent: 0,
        }
    }

    pub fn seek(&mut self, t: f64) {
        let t = t.clamp(0.0, self.log.duration());
        self.clock = t;
        self.cursor = self.log.entries.partition_point(|e| e.t < t);
        self.last_fix = self.log.entries[..self.cursor].iter().rev().find_map(|e| e.fix);
    }

    /// Advances by `dt` wall-clock seconds and returns the indices of the entries now due.
    pub fn advance(&mut self, dt: f64) -> std::ops::Range<usize> {
        let start = self.cursor;
        if !self.playing {
            return start..start;
        }
        self.clock += dt * self.speed;
        let entries = &self.log.entries;
        while self.cursor < entries.len() && entries[self.cursor].t <= self.clock {
            if let Some(f) = entries[self.cursor].fix {
                self.last_fix = Some(f);
            }
            self.cursor += 1;
        }
        let due = start..self.cursor;
        if self.cursor >= entries.len() {
            if self.looped {
                // Restart on the next tick, after the due lines are sent.
                self.cursor = 0;
                self.clock = 0.0;
            } else {
                self.playing = false;
            }
        }
        due
    }

    /// Absolute UTC time of the playback position, when the log has one.
    pub fn time_unix(&self) -> Option<f64> {
        self.log.start_unix.map(|s| s + self.clock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_block_timing() {
        let text = "\\c:1782052215*00\\$GPRMC,a\r\n\\c:1782052215*00\\$GPGGA,b\r\n\\c:1782052216*00\\$GPRMC,c\r\n";
        let log = ReplayLog::parse("t", text).unwrap();
        assert_eq!(log.timing, Timing::TagBlocks);
        let t: Vec<f64> = log.entries.iter().map(|e| e.t).collect();
        // The two lines of the first second are spread over it.
        assert_eq!(t, vec![0.0, 0.5, 1.0]);
        assert_eq!(log.entries[1].line, "$GPGGA,b");
        assert_eq!(log.start_unix, Some(1_782_052_215.0));
    }

    #[test]
    fn millisecond_tag_blocks() {
        let text = "\\s:x,c:1782052215500*00\\$GPRMC,a\n\\c:1782052216000*00\\$GPRMC,b\n";
        let log = ReplayLog::parse("t", text).unwrap();
        assert!((log.duration() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn sentence_times_with_midnight_rollover() {
        let text = "\
$GPRMC,235959.00,A,2824.000,N,08033.000,W,6.0,200.0,210626,,,A*00
!AIVDM,1,1,,A,x,0*00
$GPRMC,000000.00,A,2823.990,N,08033.005,W,6.0,200.0,220626,,,A*00
$GPRMC,000001.00,A,2823.980,N,08033.010,W,6.0,200.0,220626,,,A*00
";
        let log = ReplayLog::parse("t", text).unwrap();
        assert_eq!(log.timing, Timing::SentenceTimes);
        let t: Vec<f64> = log.entries.iter().map(|e| e.t).collect();
        assert_eq!(t, vec![0.0, 0.5, 1.0, 2.0]);
        let fix = log.entries[0].fix.unwrap();
        assert!((fix.lat - 28.4).abs() < 1e-9 && (fix.lon + 80.55).abs() < 1e-9);
        // 2026-06-21 23:59:59 UTC
        assert_eq!(log.start_unix, Some(unix_midnight(2026, 6, 21).unwrap() + 86_399.0));
    }

    #[test]
    fn untimed_logs_play_at_a_fixed_rate() {
        let log = ReplayLog::parse("t", "!AIVDM,a\n!AIVDM,b\nnot nmea\n!AIVDM,c\n").unwrap();
        assert_eq!(log.timing, Timing::FixedRate);
        assert_eq!(log.entries.len(), 3);
        assert!((log.duration() - 0.2).abs() < 1e-9);
        assert!(ReplayLog::parse("t", "hello\n").is_err());
    }

    #[test]
    fn player_sends_due_lines_and_loops() {
        let log = ReplayLog::parse("t", "\\c:100*00\\$A\n\\c:101*00\\$B\n\\c:102*00\\$C\n").unwrap();
        let mut p = Player::new(log);
        assert_eq!(p.advance(0.0), 0..1);
        assert_eq!(p.advance(0.5), 1..1);
        assert_eq!(p.advance(0.6), 1..2);
        p.speed = 10.0;
        assert_eq!(p.advance(0.1), 2..3);
        assert!(!p.playing, "stops at the end");
        p.looped = true;
        p.playing = true;
        p.seek(0.0);
        p.advance(5.0);
        assert_eq!(p.cursor, 0, "looped back");
        p.seek(1.5);
        assert_eq!(p.cursor, 2);
    }
}
