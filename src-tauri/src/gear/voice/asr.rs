//! The speech check of voice lines (design 7.4): what the recognizer heard in a cut, compared
//! with the text the line should speak. Both sides go through the same spelling rules, then
//! lose case, punctuation and spaces, and numbers read as words, so "GPS fix" heard for
//! "G.P.S. fix", "10" for "ten" and "Fail safe" for "Failsafe" all match. `core/voice_check.rs`
//! runs it on a pack's lines through the `quadcam-intel` helper.

use super::lines::Spelling;

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

/// One digit as a word.
pub fn digit_word(d: u32) -> &'static str {
    ONES[d.min(9) as usize]
}

/// A whole number as words, below a million; larger numbers digit by digit.
fn number_words(n: u64) -> String {
    fn below_thousand(n: u64, out: &mut Vec<String>) {
        if n >= 100 {
            out.push(ONES[(n / 100) as usize].into());
            out.push("hundred".into());
        }
        let r = n % 100;
        if r == 0 && n >= 100 {
            return;
        }
        if r < 20 {
            out.push(ONES[r as usize].into());
        } else {
            out.push(TENS[(r / 10) as usize].into());
            if !r.is_multiple_of(10) {
                out.push(ONES[(r % 10) as usize].into());
            }
        }
    }
    if n >= 1_000_000 {
        return n
            .to_string()
            .chars()
            .map(|c| ONES[c.to_digit(10).unwrap_or(0) as usize])
            .collect::<Vec<_>>()
            .join(" ");
    }
    let mut out = Vec::new();
    if n >= 1000 {
        below_thousand(n / 1000, &mut out);
        out.push("thousand".into());
        if n.is_multiple_of(1000) {
            return out.join(" ");
        }
    }
    below_thousand(n % 1000, &mut out);
    out.join(" ")
}

/// Digits as words: `3.7` is "three point seven", `1,500` is "one thousand five hundred".
fn spell_numbers(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        // A point read before a number or a word, as the recognizer writes "point five":
        // ".5", ".one". Not inside a word: "G.P.S." keeps its dots.
        if chars[i] == '.'
            && chars.get(i + 1).is_some_and(|c| c.is_alphanumeric())
            && (i == 0 || !chars[i - 1].is_alphanumeric())
        {
            out.push_str(" point ");
            i += 1;
            continue;
        }
        if !chars[i].is_ascii_digit() {
            match chars[i] {
                '%' => out.push_str(" percent "),
                '&' => out.push_str(" and "),
                c => out.push(c),
            }
            i += 1;
            continue;
        }
        let mut whole = String::new();
        while i < chars.len()
            && (chars[i].is_ascii_digit()
                || chars[i] == ','
                    && !whole.is_empty()
                    && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()))
        {
            if chars[i] != ',' {
                whole.push(chars[i]);
            }
            i += 1;
        }
        out.push(' ');
        out.push_str(&number_words(whole.parse().unwrap_or(0)));
        if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
            out.push_str(" point");
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                out.push(' ');
                out.push_str(ONES[chars[i].to_digit(10).unwrap_or(0) as usize]);
                i += 1;
            }
        }
        out.push(' ');
    }
    out
}

/// The text as the check compares it: the spelling rules applied, numbers as words, then
/// only its letters and digits, lowercase.
pub fn normalise(text: &str, spelling: &Spelling) -> String {
    spell_numbers(&spelling.apply(text))
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Words the recognizer writes as another word that sounds the same, as (the line's word,
/// what was heard). Each pair is a true sound-alike: a heard word counts as the line's word
/// only when the line has that word. A word that sounds different ("blocked" for "locked",
/// "Boilers" for "Spoilers") is never here: that take is wrong.
const SOUND_ALIKES: &[(&str, &str)] = &[
    ("amp", "am"),
    ("hertz", "hurts"),
    ("high", "hi"),
    ("idle", "idol"),
    ("knot", "not"),
    ("lift", "lyft"),
    ("radians", "radiance"),
    ("variance", "variants"),
    ("watt", "what"),
];

/// The words of a text as the check compares them: the spelling rules, numbers as words, each
/// word only its letters and digits, lowercase.
pub fn words(text: &str, spelling: &Spelling) -> Vec<String> {
    spell_numbers(&spelling.apply(text))
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Whether what the recognizer heard is the line's spoken text. A heard word that sounds
/// like one of the line's words (`SOUND_ALIKES`) counts as that word, and a line that starts
/// with "point" may be heard with a zero before it ("0.5" for "point five").
pub fn same(spoken: &str, heard: &str, spelling: &Spelling) -> bool {
    let want = normalise(spoken, spelling);
    if want.is_empty() {
        return false;
    }
    let got = normalise(heard, spelling);
    if want == got {
        return true;
    }
    let mine = words(spoken, spelling);
    let got: String = words(heard, spelling)
        .into_iter()
        .map(|w| {
            SOUND_ALIKES
                .iter()
                .find(|(line, alike)| *alike == w && mine.iter().any(|m| m == line))
                .map_or(w, |(line, _)| line.to_string())
        })
        .collect();
    want == got || (want.starts_with("point") && got.strip_prefix("zero") == Some(&want))
}

/// The reason a re-take gives for a line the check heard wrong.
pub fn reason(heard: &str) -> String {
    if heard.trim().is_empty() {
        "The speech check heard nothing.".into()
    } else {
        format!("The speech check heard {:?}.", heard.trim())
    }
}

/// Whether a re-take's reason came from the speech check.
pub fn is_speech_reason(reason: &str) -> bool {
    reason.starts_with("The speech check heard")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_as_words() {
        assert_eq!(number_words(0), "zero");
        assert_eq!(number_words(10), "ten");
        assert_eq!(number_words(21), "twenty one");
        assert_eq!(number_words(100), "one hundred");
        assert_eq!(number_words(115), "one hundred fifteen");
        assert_eq!(number_words(1500), "one thousand five hundred");
        assert_eq!(number_words(2000), "two thousand");
        let words = |t: &str| {
            spell_numbers(t)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(words("3.7 volts"), "three point seven volts");
        assert_eq!(words("1,500"), "one thousand five hundred");
        assert_eq!(words("50%"), "fifty percent");
        assert_eq!(words("A, B"), "A, B");
    }

    #[test]
    fn heard_text_matches_its_line_whatever_the_form() {
        let sp = Spelling::builtin().unwrap();
        let spoken = |t: &str| sp.apply(t);
        assert!(same(&spoken("Armed"), "Armed.", &sp));
        assert!(same(&spoken("Failsafe"), "Fail safe.", &sp));
        assert!(same(&spoken("ten"), "10.", &sp));
        assert!(same(&spoken("GPS fix"), "GPS fix.", &sp));
        assert!(same(&spoken("GPS fix"), "G.P.S. fix", &sp));
        assert!(same(&spoken("GPS fix"), "G P S fix", &sp));
        assert!(same(&spoken("Link quality low"), "link quality, low!", &sp));
        assert!(same(&spoken("Capacity warning"), "capacity warning.", &sp));
        assert!(same(&spoken("RSSI low"), "RSSI low", &sp));
        assert!(same(&spoken("one hundred"), "100", &sp));
        assert!(!same(&spoken("Armed"), "Unarmed.", &sp));
        assert!(!same(&spoken("Battery low"), "Battery.", &sp));
        assert!(!same(&spoken("six"), "", &sp));
        assert!(!same("", "", &sp));
    }

    #[test]
    fn a_sound_alike_of_the_lines_word_passes() {
        let sp = Spelling::builtin().unwrap();
        let spoken = |t: &str| sp.apply(t);
        assert!(same(&spoken("hertz"), "Hurts.", &sp));
        assert!(same(&spoken("watt"), "What?", &sp));
        assert!(same(&spoken("high"), "Hi.", &sp));
        assert!(same(&spoken("amp"), "Am.", &sp));
        assert!(same(&spoken("EKF variance"), "EKF variants.", &sp));
        assert!(same(
            &spoken("Reverse Thrust Idle"),
            "Reverse thrust idol.",
            &sp
        ));
        assert!(same(&spoken("Lift"), "Lyft.", &sp));
        assert!(same(&spoken("radians"), "Radiance.", &sp));
        assert!(same(&spoken("knot"), "Not.", &sp));
        // Only for the word the line has.
        assert!(!same(&spoken("watts"), "What?", &sp));
        assert!(!same(&spoken("knot"), "What?", &sp));
        assert!(!same(&spoken("Armed"), "Hi.", &sp));
    }

    #[test]
    fn a_point_is_heard_as_a_dot() {
        let sp = Spelling::builtin().unwrap();
        let spoken = |t: &str| sp.apply(t);
        assert!(same(&spoken("point five"), ".5.", &sp));
        assert!(same(&spoken("point one"), ".one.", &sp));
        assert!(same(&spoken("point zero"), "0.0.", &sp));
        assert!(same(&spoken("point two"), "0.2", &sp));
        assert!(!same(&spoken("point two"), ".3.", &sp));
        assert!(!same(&spoken("two"), "0.2", &sp));
        assert!(same(&spoken("GPS fix"), "G.P.S. fix", &sp));
    }

    #[test]
    fn a_word_that_sounds_different_still_fails() {
        let sp = Spelling::builtin().unwrap();
        let spoken = |t: &str| sp.apply(t);
        for (line, heard) in [
            ("Spoilers down", "Boilers down."),
            ("Spoilers up", "Boilers up."),
            ("Spooling up", "Booling up."),
            ("Locked", "blocked."),
            ("Angle off", "Angle loft."),
            ("Angle on", "Hang along."),
            ("RF mode", "Ref mode."),
            ("Diffs locked", "Diff's lock."),
            ("hertz", "Turfs."),
            ("knots", "Not."),
            ("low", "Blow."),
            ("Half pack", "It's half pack."),
            ("radians", "Indians."),
            ("Reverse Thrust", "Verse thrust."),
            ("milliamps", "Billy amps."),
            ("Max gyro", "Axe gyro."),
            ("minus", "Highness."),
            ("no", "Oh."),
            ("thirty", "Oh, see."),
        ] {
            assert!(!same(&spoken(line), heard, &sp), "{line:?} heard {heard:?}");
        }
    }

    #[test]
    fn a_speech_reason_names_what_was_heard() {
        assert_eq!(reason(" Unarmed. "), "The speech check heard \"Unarmed.\".");
        assert_eq!(reason(""), "The speech check heard nothing.");
        assert!(is_speech_reason(&reason("x")));
        assert!(!is_speech_reason("the cut is silent"));
    }
}
