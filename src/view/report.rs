use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Formatter;
use std::ops::Add;

use crate::data::activity::{self, Activity};
use crate::view::format_util;
use chrono::Duration;
use nu_ansi_term::Style;

use super::format_util::format_duration;

type ProjectMap = BTreeMap<String, ReportEntry>;
type ReportLines = Vec<ReportLine>;

struct Report {
    project_map: ProjectMap,
    total_duration: Duration,
}

#[derive(Debug)]
struct ReportEntry {
    total_duration: Duration,
    items: ProjectMap,
}

impl ReportEntry {
    fn new() -> Self {
        ReportEntry {
            total_duration: Duration::zero(),
            items: BTreeMap::new(),
        }
    }
}

enum ReportLine {
    Item(ReportLineItem),
    Separator,
}

impl ReportLine {
    fn new_report_line(indent: usize, name: String, heading: bool, duration: Duration) -> Self {
        ReportLine::Item(ReportLineItem {
            indent: indent,
            name: name,
            heading: heading,
            duration: duration,
        })
    }

    fn new_separator() -> Self {
        Self::Separator
    }

    fn write_line(
        &self,
        f: &mut Formatter,
        longest_line_info: &LongestLineInfo,
    ) -> Result<(), std::fmt::Error> {
        match self {
            Self::Item(line) => {
                let style = if line.heading {
                    Style::new().bold()
                } else {
                    Style::new()
                };
                writeln!(f, "{}", style.paint(line.as_string(longest_line_info)))
            }
            Self::Separator => writeln!(f),
        }
    }
}

struct ReportLineItem {
    indent: usize,
    name: String,
    heading: bool,
    duration: Duration,
}

impl ReportLineItem {
    fn as_string(&self, longest_line_info: &LongestLineInfo) -> String {
        format!(
            "{indent}{name:.<name_width$}\t{duration:>duration_width$}",
            indent = " ".repeat(self.indent * 2),
            name = self.name,
            duration = format_util::format_duration(&self.duration),
            duration_width = longest_line_info.duration,
            name_width = longest_line_info.name
        )
    }
}

impl Report {
    fn new(activities: &[&activity::Activity], groups: Vec<Box<dyn ReportGroup>>) -> Report {
        Report {
            project_map: create_project_map(activities, groups),
            total_duration: sum_duration(activities),
        }
    }

    fn return_report_lines(&self) -> ReportLines {
        let mut lines: ReportLines = Vec::new();

        recursively_return_lines(&self.project_map, &mut lines, 0);

        fn recursively_return_lines(map: &ProjectMap, lines: &mut ReportLines, indent: usize) {
            for (name, entry) in map.iter() {
                lines.push(ReportLine::new_report_line(
                    indent.clone(),
                    name.clone(),
                    !entry.items.is_empty(), //Consider the line a heading if the map doesn't contain any items
                    entry.total_duration,
                ));

                recursively_return_lines(&entry.items, lines, indent.clone() + 1);
            }

            //TODO work out why there is only ever one instance of indent 0.
            //This suggests that the first level only contains one item which might be an indication that something else is wrong
            if 1 <= indent && indent <= 2 {
                lines.push(ReportLine::new_separator());
            }
        }

        lines
    }
}

impl<'a> fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lines = self.return_report_lines();
        let longest_line_info = get_longest_line_info(&lines);

        for line in lines {
            line.write_line(f, &longest_line_info)?;
        }

        Ok(())
    }
}

pub trait ReportGroup {
    fn return_identifier(&self, activity: &Activity) -> String;
}

pub struct ReportGroupDate;
impl ReportGroup for ReportGroupDate {
    fn return_identifier(&self, activity: &Activity) -> String {
        activity.start.date().to_string()
    }
}
pub struct ReportGroupProject;
impl ReportGroup for ReportGroupProject {
    fn return_identifier(&self, activity: &Activity) -> String {
        activity.project.to_string()
    }
}
pub struct ReportGroupDescription;
impl ReportGroup for ReportGroupDescription {
    fn return_identifier(&self, activity: &Activity) -> String {
        activity.description.to_string()
    }
}

pub fn show_activities<'a>(
    activities: &'a [&'a activity::Activity],
    groups: Vec<Box<dyn ReportGroup>>,
) {
    let report = Report::new(activities, groups);
    println!("\n{report}");
}

fn create_project_map<'a>(
    activities: &'a [&'a activity::Activity],
    groups: Vec<Box<dyn ReportGroup>>,
) -> ProjectMap {
    fn recursively_apply_group(
        project_map: &mut ProjectMap,
        groups: &[Box<dyn ReportGroup>],
        activity: &Activity,
    ) {
        let group = &groups[0];
        let identifier = group.return_identifier(activity);
        let report_entry = project_map
            .entry(identifier)
            .or_insert_with(|| ReportEntry::new());

        report_entry.total_duration = report_entry.total_duration.add(activity.get_duration());

        match groups.len() {
            0 => panic!("length of group is {}", groups.len()),
            1 => return,
            2.. => recursively_apply_group(&mut report_entry.items, &groups[1..], activity),
        }
    }

    let mut project_map: ProjectMap = BTreeMap::new();

    for a in activities {
        recursively_apply_group(&mut project_map, &groups, a);
    }

    project_map
}

pub fn sum_duration(activities: &[&activity::Activity]) -> Duration {
    let mut duration = Duration::seconds(0);

    for activity in activities {
        duration = duration.add(activity.get_duration());
    }

    duration
}

struct LongestLineInfo {
    name: usize,
    duration: usize,
}

fn return_name_len(line: &ReportLine) -> usize {
    if let ReportLine::Item(item) = line {
        item.name.chars().count() + item.indent
    } else {
        0
    }
}

fn return_duration_len(line: &ReportLine) -> usize {
    if let ReportLine::Item(item) = line {
        format_duration(&item.duration).chars().count()
    } else {
        0
    }
}

fn get_longest_line_info(lines: &[ReportLine]) -> LongestLineInfo {
    let longest_name = lines
        .iter()
        .map(|line| return_name_len(line))
        .max()
        .unwrap_or(0);
    let longest_duration = lines
        .iter()
        .map(|line| return_duration_len(line))
        .max()
        .unwrap_or(0);

    LongestLineInfo {
        name: longest_name,
        duration: longest_duration,
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDateTime;

    use super::*;

    #[test]
    fn sum_duration_test() {
        let mut activities: Vec<&activity::Activity> = Vec::new();
        assert_eq!(sum_duration(&activities).num_seconds(), 0);

        let mut a1 = activity::Activity::start(
            "p1".to_string(),
            "d1".to_string(),
            Some(
                NaiveDateTime::parse_from_str("2021-09-01 15:00:00", "%Y-%m-%d %H:%M:%S").unwrap(),
            ),
        );
        a1.end = Some(
            NaiveDateTime::parse_from_str("2021-09-01 15:20:00", "%Y-%m-%d %H:%M:%S").unwrap(),
        ); // 20 * 60 = 1,200 seconds
        let mut a2 = activity::Activity::start(
            "p1".to_string(),
            "d2".to_string(),
            Some(
                NaiveDateTime::parse_from_str("2021-09-01 15:21:00", "%Y-%m-%d %H:%M:%S").unwrap(),
            ),
        );
        a2.end = Some(
            NaiveDateTime::parse_from_str("2021-09-01 16:21:00", "%Y-%m-%d %H:%M:%S").unwrap(),
        ); // 60 * 60 = 3,600 seconds
        let mut a3 = activity::Activity::start(
            "p2".to_string(),
            "d1".to_string(),
            Some(
                NaiveDateTime::parse_from_str("2021-09-01 16:21:00", "%Y-%m-%d %H:%M:%S").unwrap(),
            ),
        );
        a3.end = Some(
            NaiveDateTime::parse_from_str("2021-09-02 16:21:00", "%Y-%m-%d %H:%M:%S").unwrap(),
        ); // 24 * 60 * 60 = 86,400 seconds

        activities.push(&a1);
        activities.push(&a2);
        activities.push(&a3);

        assert_eq!(sum_duration(&activities).num_seconds(), 91200);
    }

    #[test]
    fn group_activities_by_project_test() {
        let a1 = activity::Activity::start("p1".to_string(), "d1".to_string(), None);
        let a2 = activity::Activity::start("p1".to_string(), "d2".to_string(), None);
        let a3 = activity::Activity::start("p2".to_string(), "d1".to_string(), None);

        let activities = vec![&a1, &a2, &a3];
        let m = create_project_map(
            &activities,
            vec![
                Box::new(ReportGroupProject),
                Box::new(ReportGroupDescription),
            ],
        );

        assert_eq!(m.len(), 2);
        assert_eq!(m.get("p1").unwrap().items.len(), 2, "{m:?}");
        assert_eq!(m.get("p2").unwrap().items.len(), 1);
    }
}
