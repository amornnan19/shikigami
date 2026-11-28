use super::Member;

const BOUNDARY_PUNCTUATION: [char; 8] = [',', '.', ':', ';', '!', '?', ')', ']'];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandOff {
    pub agent_id: String,
    pub task: String,
}

struct Scan {
    chars: Vec<char>,
    ignored: Vec<bool>,
}

fn run_length(chars: &[char], from: usize) -> usize {
    chars[from..].iter().take_while(|c| **c == '`').count()
}

fn mark_inline_code(chars: &[char], start: usize, end: usize, ignored: &mut [bool]) {
    let mut at = start;
    while at < end {
        if chars[at] != '`' {
            at += 1;
            continue;
        }
        let opening = run_length(&chars[..end], at);
        let mut search = at + opening;
        let mut closing = None;
        while search < end {
            if chars[search] == '`' {
                let length = run_length(&chars[..end], search);
                if length == opening {
                    closing = Some(search + length);
                    break;
                }
                search += length;
            } else {
                search += 1;
            }
        }
        match closing {
            Some(stop) => {
                ignored[at..stop].fill(true);
                at = stop;
            }
            None => at += opening,
        }
    }
}

fn line_ranges(chars: &[char]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    loop {
        let end = chars[start..]
            .iter()
            .position(|c| *c == '\n')
            .map_or(chars.len(), |offset| start + offset);
        ranges.push((start, end));
        if end >= chars.len() {
            return ranges;
        }
        start = end + 1;
    }
}

fn fence_of(line: &str) -> Option<(char, usize, &str)> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let length = trimmed.chars().take_while(|c| *c == marker).count();
    (length >= 3).then(|| (marker, length, &trimmed[length..]))
}

impl Scan {
    fn new(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let mut ignored = vec![false; chars.len()];
        let mut open: Option<(char, usize)> = None;
        for (start, end) in line_ranges(&chars) {
            let line: String = chars[start..end].iter().collect();
            let fence = fence_of(&line);
            match (open, fence) {
                (Some((marker, length)), Some((found, run, rest)))
                    if found == marker && run >= length && rest.trim().is_empty() =>
                {
                    ignored[start..end].fill(true);
                    open = None;
                }
                (Some(_), _) => ignored[start..end].fill(true),
                (None, Some((marker, run, _))) => {
                    ignored[start..end].fill(true);
                    open = Some((marker, run));
                }
                (None, None) => mark_inline_code(&chars, start, end, &mut ignored),
            }
        }
        Self { chars, ignored }
    }

    fn mention_at<'a>(&self, at: usize, members: &'a [Member]) -> Option<(usize, &'a Member)> {
        if self.chars.get(at) != Some(&'@') || self.ignored[at] {
            return None;
        }
        if at > 0 && self.chars[at - 1].is_alphanumeric() {
            return None;
        }
        let mut best: Option<(usize, &Member)> = None;
        for member in members {
            let name: Vec<char> = member.name.chars().collect();
            let end = at + 1 + name.len();
            if name.is_empty() || end > self.chars.len() {
                continue;
            }
            let matches = name
                .iter()
                .zip(&self.chars[at + 1..end])
                .all(|(wanted, found)| wanted.to_lowercase().eq(found.to_lowercase()));
            let boundary = self
                .chars
                .get(end)
                .is_none_or(|c| c.is_whitespace() || BOUNDARY_PUNCTUATION.contains(c));
            let clear = !self.ignored[at + 1..end].iter().any(|flag| *flag);
            if matches && boundary && clear && best.is_none_or(|(longest, _)| end > longest) {
                best = Some((end, member));
            }
        }
        best
    }
}

pub fn addressed(text: &str, members: &[Member]) -> Vec<String> {
    let scan = Scan::new(text);
    let mut found: Vec<String> = Vec::new();
    let mut at = 0;
    while at < scan.chars.len() {
        match scan.mention_at(at, members) {
            Some((end, member)) => {
                if !found.contains(&member.agent_id) {
                    found.push(member.agent_id.clone());
                }
                at = end;
            }
            None => at += 1,
        }
    }
    found
}

pub fn hand_offs(reply: &str, members: &[Member], own_agent_id: &str) -> Vec<HandOff> {
    let scan = Scan::new(reply);
    let mut found: Vec<HandOff> = Vec::new();
    let mut current: Option<(String, String)> = None;
    let mut close = |current: &mut Option<(String, String)>| {
        if let Some((agent_id, task)) = current.take() {
            let task = task.trim().to_owned();
            if !task.is_empty() {
                found.push(HandOff { agent_id, task });
            }
        }
    };
    for (start, end) in line_ranges(&scan.chars) {
        let first = (start..end).find(|at| !scan.chars[*at].is_whitespace());
        let target = first
            .and_then(|at| scan.mention_at(at, members))
            .filter(|(_, member)| member.agent_id != own_agent_id);
        match target {
            Some((stop, member)) => {
                close(&mut current);
                let rest: String = scan.chars[stop..end].iter().collect();
                let rest =
                    rest.trim_start_matches(|c: char| c.is_whitespace() || ",:;".contains(c));
                current = Some((member.agent_id.clone(), rest.to_owned()));
            }
            None => {
                if let Some((_, task)) = current.as_mut() {
                    task.push('\n');
                    task.extend(&scan.chars[start..end]);
                }
            }
        }
    }
    close(&mut current);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, name: &str) -> Member {
        Member {
            agent_id: id.to_owned(),
            name: name.to_owned(),
            color: "red".to_owned(),
        }
    }

    fn team() -> Vec<Member> {
        vec![
            member("a", "Ada"),
            member("b", "Bo"),
            member("t", "Test"),
            member("ta", "Test Agent"),
        ]
    }

    fn ids(text: &str) -> Vec<String> {
        addressed(text, &team())
    }

    fn offs(reply: &str, own: &str) -> Vec<(String, String)> {
        hand_offs(reply, &team(), own)
            .into_iter()
            .map(|hand_off| (hand_off.agent_id, hand_off.task))
            .collect()
    }

    #[test]
    fn a_name_matches_without_regard_to_case() {
        assert_eq!(ids("@bo list the files"), ["b"]);
        assert_eq!(ids("@BO list the files"), ["b"]);
        assert_eq!(ids("@aDa hi"), ["a"]);
    }

    #[test]
    fn the_longest_member_name_wins_at_a_mention() {
        assert_eq!(ids("@Test Agent look"), ["ta"]);
        assert_eq!(ids("@test agent look"), ["ta"]);
        assert_eq!(ids("@Test look"), ["t"]);
        assert_eq!(ids("@Test Agentx look"), ["t"]);
        assert_eq!(ids("@Test Ag"), ["t"]);
    }

    #[test]
    fn the_name_ends_at_the_end_of_the_text_whitespace_or_listed_punctuation() {
        assert_eq!(ids("@Bo"), ["b"]);
        assert_eq!(ids("@Bo\nnext"), ["b"]);
        for end in [",", ".", ":", ";", "!", "?", ")", "]"] {
            assert_eq!(ids(&format!("(see @Bo{end} ok")), ["b"], "{end}");
        }
        for glued in ["x", "1", "-", "'", "@", "}"] {
            assert!(ids(&format!("@Bo{glued} ok")).is_empty(), "{glued}");
        }
    }

    #[test]
    fn a_mention_glued_to_a_word_before_it_is_plain_text() {
        assert!(ids("mail me at ada@Bo.com").is_empty());
        assert_eq!(ids("(@Bo) and @Ada."), ["b", "a"]);
    }

    #[test]
    fn fenced_code_and_inline_code_are_ignored() {
        assert!(ids("```\n@Bo hidden\n```").is_empty());
        assert!(ids("```sh\n@Bo hidden").is_empty());
        assert!(ids("run `ask @Bo please` now").is_empty());
        assert!(ids("run ``a ` @Bo please`` now").is_empty());
        assert_eq!(ids("run `ask @Bo please` now @Ada"), ["a"]);
        assert_eq!(ids("```\n@Bo hidden\n```\n@Ada visible"), ["a"]);
        assert_eq!(ids("`code` then @Bo"), ["b"]);
        assert_eq!(ids("a lone ` backtick then @Bo"), ["b"]);
    }

    #[test]
    fn a_fence_closes_only_on_the_same_character_and_at_least_the_opening_length() {
        assert!(ids("~~~\n@Bo hidden\n~~~").is_empty());
        assert!(ids("~~~\n```\n@Bo hidden\n```\n~~~").is_empty());
        assert!(ids("```\n~~~\n@Bo hidden\n```").is_empty());
        assert!(ids("````\n```\n@Bo hidden\n```\n````").is_empty());
        assert!(ids("```\n@Bo hidden\n```x\n@Bo still hidden").is_empty());
        assert_eq!(ids("~~~\n@Bo hidden\n~~~\n@Ada shown"), ["a"]);
        assert_eq!(ids("````\n@Bo hidden\n````\n@Ada shown"), ["a"]);
    }

    #[test]
    fn a_name_that_is_not_a_member_is_plain_text() {
        assert!(ids("@Zed do this").is_empty());
        assert!(ids("@ Bo").is_empty());
        assert!(ids("no mention").is_empty());
    }

    #[test]
    fn several_mentions_come_in_order_of_first_appearance_without_duplicates() {
        assert_eq!(ids("@Bo then @Ada, then @bo again"), ["b", "a"]);
        assert_eq!(ids("@Ada @Test Agent @Test"), ["a", "ta", "t"]);
    }

    #[test]
    fn a_hand_off_line_starts_with_a_mention_of_another_member() {
        assert_eq!(
            offs("Done with my part.\n@Bo check the README", "a"),
            [("b".to_owned(), "check the README".to_owned())]
        );
        assert_eq!(
            offs("   @Bo: check it", "a"),
            [("b".to_owned(), "check it".to_owned())]
        );
        assert!(offs("I asked @Bo to check", "a").is_empty());
    }

    #[test]
    fn a_task_runs_over_the_following_lines_up_to_the_next_hand_off_line() {
        let reply = "Plan:\n@Bo check the README\nand the license\n\n@Test Agent run the tests\nthen report\nthanks";
        assert_eq!(
            offs(reply, "a"),
            [
                (
                    "b".to_owned(),
                    "check the README\nand the license".to_owned()
                ),
                (
                    "ta".to_owned(),
                    "run the tests\nthen report\nthanks".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn a_line_that_mentions_the_replying_agent_is_not_a_hand_off() {
        assert!(offs("@Ada do it", "a").is_empty());
        assert_eq!(
            offs("@Bo first\n@Ada not me\nstill first", "a"),
            [("b".to_owned(), "first\n@Ada not me\nstill first".to_owned())]
        );
        assert_eq!(
            offs("@Test Agent go", "t"),
            [("ta".to_owned(), "go".to_owned())]
        );
        assert!(offs("@Test Agent go", "ta").is_empty());
    }

    #[test]
    fn lines_in_code_blocks_and_non_members_are_not_hand_offs() {
        assert!(offs("```\n@Bo check\n```", "a").is_empty());
        assert!(offs("@Zed check", "a").is_empty());
        assert_eq!(
            offs("@Bo run this:\n```\n@Ada not a hand-off\n```\ndone", "a"),
            [(
                "b".to_owned(),
                "run this:\n```\n@Ada not a hand-off\n```\ndone".to_owned()
            )]
        );
    }

    #[test]
    fn a_hand_off_with_no_task_is_dropped() {
        assert!(offs("@Bo", "a").is_empty());
        assert!(offs("@Bo   \n  ", "a").is_empty());
        assert_eq!(
            offs("@Bo\nthe task is here", "a"),
            [("b".to_owned(), "the task is here".to_owned())]
        );
    }

    #[test]
    fn the_shared_vectors_give_the_same_mentions_as_the_ui_mirror() {
        let vectors: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../tests/fixtures/mention-vectors.json"))
                .unwrap();
        for vector in vectors {
            let text = vector["text"].as_str().unwrap();
            let names = |key: &str| -> Vec<String> {
                vector[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|name| name.as_str().unwrap().to_owned())
                    .collect()
            };
            let members: Vec<Member> = names("members")
                .iter()
                .map(|name| member(name, name))
                .collect();
            assert_eq!(addressed(text, &members), names("mentions"), "{text:?}");
        }
    }
}
