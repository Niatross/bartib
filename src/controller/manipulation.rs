use anyhow::{anyhow, bail, Context, Error, Result};
use chrono::NaiveDateTime;
use std::process::Command;

use crate::conf;
use crate::data::activity;
use crate::data::bartib_file;
use crate::data::getter;
use crate::data::getter::get_running_activities;
use crate::view::format_util;

// starts a new activity
pub fn start(
    file_name: &str,
    project_name: &str,
    activity_description: &str,
    time: Option<NaiveDateTime>,
) -> Result<()> {
    let mut file_content: Vec<bartib_file::Line> = Vec::new();

    if let Ok(mut previous_file_content) = bartib_file::get_file_content(file_name) {
        // if we start a new activities programaticly, we stop all other activities first.
        // However, we must not assume that there is always only one activity
        // running as the user may have started activities manually
        stop_all_running_activities(&mut previous_file_content, time);

        file_content.append(&mut previous_file_content);
    }

    let activity = activity::Activity::start(
        project_name.to_string(),
        activity_description.to_string(),
        time,
    );

    save_new_activity(file_name, &mut file_content, activity)
}

fn save_new_activity(
    file_name: &str,
    file_content: &mut Vec<bartib_file::Line>,
    activity: activity::Activity,
) -> Result<(), Error> {
    println!(
        "Started activity: \"{}\" ({}) at {}",
        activity.description,
        activity.project,
        activity.start.format(conf::FORMAT_DATETIME)
    );

    file_content.push(bartib_file::Line::for_activity(activity));
    bartib_file::write_to_file(file_name, file_content)
        .context(format!("Could not write to file: {file_name}"))
}

pub fn change(
    file_name: &str,
    project_name: Option<&str>,
    activity_description: Option<&str>,
    time: Option<NaiveDateTime>,
) -> Result<()> {
    let mut file_content = bartib_file::get_file_content(file_name)?;

    let mut prev_start_time: Option<NaiveDateTime> = None;

    // iterate through all activities a check whether they need modifying
    for line in &mut file_content {
        if let Ok(activity) = &mut line.activity {
            if !activity.is_stopped() {
                // only modify currently running activities
                let mut changed = false;

                if let Some(project_name) = project_name {
                    activity.project = project_name.to_string();
                    changed = true;
                }

                if let Some(activity_description) = activity_description {
                    activity.description = activity_description.to_string();
                    changed = true;
                }

                if let Some(time) = time {
                    prev_start_time = Some(activity.start.clone());
                    activity.start = time;
                    changed = true;
                }

                if changed {
                    println!(
                        "Changed activity: \"{}\" ({}) started at {}",
                        activity.description,
                        activity.project,
                        activity.start.format(conf::FORMAT_DATETIME)
                    );
                    line.set_changed();
                }
            }
        }
    }

    if let Some(prev_start_time) = prev_start_time {
        // if the user is changing the start time, check to see if there is another entry with the same finish time
        // If there is, also change that finish time to the new user entered start time
        // This is useful where the user has stopped a task by starting a new one

        // This does not handle their being more than one active project
        // As this is not directly supported by the bartib API it has been ignored
        // instead this will just update the last active project
        // Consideration should be made to whether there should be guarding to protect against this eventuality

        file_content
            .iter_mut()
            .filter(|line| {
                line.activity.as_ref().map_or(false, |activity| {
                    activity
                        .end
                        .map_or(false, |end_time| end_time == prev_start_time)
                })
            })
            .for_each(|line| {
                let mut activity = line.activity.as_ref().unwrap().clone();
                activity.end = Some(time.unwrap().clone());

                println!(
                    "Changed activity: \"{}\" ({}) ended at {}",
                    activity.description,
                    activity.project,
                    time.unwrap().format(conf::FORMAT_DATETIME)
                );

                line.activity = Ok(activity);
                line.set_changed();
            });
    }

    bartib_file::write_to_file(file_name, &file_content)
        .context(format!("Could not write to file: {file_name}"))
}

// stops all currently running activities
pub fn stop(file_name: &str, time: Option<NaiveDateTime>) -> Result<()> {
    let mut file_content = bartib_file::get_file_content(file_name)?;
    stop_all_running_activities(&mut file_content, time);
    bartib_file::write_to_file(file_name, &file_content)
        .context(format!("Could not write to file: {file_name}"))
}

// cancels all currently running activities
pub fn cancel(file_name: &str) -> Result<()> {
    let file_content = bartib_file::get_file_content(file_name)?;
    let mut new_file_content: Vec<bartib_file::Line> = Vec::new();

    for line in file_content {
        match &line.activity {
            Ok(activity) => {
                if activity.is_stopped() {
                    new_file_content.push(line);
                } else {
                    println!(
                        "Canceled activity: \"{}\" ({}) started at {}",
                        activity.description,
                        activity.project,
                        activity.start.format(conf::FORMAT_DATETIME)
                    );
                }
            }
            Err(_) => new_file_content.push(line),
        }
    }

    bartib_file::write_to_file(file_name, &new_file_content)
        .context(format!("Could not write to file: {file_name}"))
}

// continue last activity
pub fn continue_last_activity(
    file_name: &str,
    project_name: Option<&str>,
    activity_description: Option<&str>,
    time: Option<NaiveDateTime>,
    number: usize,
) -> Result<()> {
    let mut file_content = bartib_file::get_file_content(file_name)?;

    let descriptions_and_projects: Vec<(&String, &String)> =
        getter::get_descriptions_and_projects(&file_content);

    if descriptions_and_projects.is_empty() {
        bail!("No activity has been started before.")
    }

    if number > descriptions_and_projects.len() {
        bail!(format!(
            "Less than {} distinct activities have been logged yet",
            number
        ));
    }

    let i = descriptions_and_projects
        .len()
        .saturating_sub(number)
        .saturating_sub(1);
    let optional_description_and_project = descriptions_and_projects.get(i);

    if let Some((description, project)) = optional_description_and_project {
        let new_activity = activity::Activity::start(
            project_name.unwrap_or(project).to_string(),
            activity_description.unwrap_or(description).to_string(),
            time,
        );
        stop_all_running_activities(&mut file_content, time);
        save_new_activity(file_name, &mut file_content, new_activity)
    } else {
        bail!(format!(
            "Less than {} distinct activities have been logged yet",
            number
        ));
    }
}

pub fn return_continue_current_activity_closure(
    file_name: &str,
) -> Result<Box<dyn Fn() -> Result<()> + '_>> {
    let file_content = bartib_file::get_file_content(&file_name)?;

    let current_activities = get_running_activities(&file_content);

    let closure: Box<dyn Fn() -> Result<()> + '_> = match current_activities.len() {
        0 => Box::new(|| -> Result<()> {
            println!("There is no activity currently running to restart");
            Ok(())
        }),
        1 => {
            let activity = current_activities[0].clone();
            Box::new(move || start(file_name, &activity.project, &activity.description, None))
        }
        _ => {
            return Err(anyhow!(
                "The continue flag does not support multiple activties running at once"
            ))
        }
    };

    Ok(closure)
}

pub fn start_editor(file_name: &str, optional_editor_command: Option<&str>) -> Result<()> {
    let editor_command = optional_editor_command.context("editor command is missing")?;
    let command = Command::new(editor_command).arg(file_name).spawn();

    match command {
        Ok(mut child) => {
            child.wait().context("editor did not execute")?;
            Ok(())
        }
        Err(e) => Err(anyhow!(e)),
    }
}

fn stop_all_running_activities(
    file_content: &mut [bartib_file::Line],
    time: Option<NaiveDateTime>,
) {
    for line in file_content {
        if let Ok(activity) = &mut line.activity {
            if !activity.is_stopped() {
                activity.stop(time);
                println!(
                    "Stopped activity: \"{}\" ({}) started at {} ({})",
                    activity.description,
                    activity.project,
                    activity.start.format(conf::FORMAT_DATETIME),
                    format_util::format_duration(&activity.get_duration()),
                );

                line.set_changed();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, str::FromStr, time::Duration};

    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    use temp_dir::TempDir;

    use crate::data::bartib_file::{get_file_content, Line};

    fn seed_test_file(
        file_path: &str,
        additional_seed_activities: Vec<(&str, &str, Option<NaiveDateTime>, Option<NaiveDateTime>)>,
    ) -> Vec<Line> {
        // Seed the temp file with random activities

        let mut count = 0;

        for i in 1..100 {
            let project = format!("proj{i}");
            let description = format!("desc{i}");
            super::start(file_path, project.as_str(), description.as_str(), None).unwrap();
            super::stop(file_path, None).unwrap();
            count += 1;
        }

        // write additional activities for testing specific edge conditions
        for activity in additional_seed_activities {
            super::start(file_path, activity.0, activity.1, activity.2).unwrap();
            super::stop(file_path, activity.3).unwrap();
            count += 1;
        }

        let mut file_contents = get_file_content(file_path).unwrap();
        let added_lines: Vec<Line> = file_contents
            .drain((file_contents.len() - count)..)
            .collect();
        added_lines
    }

    #[test]
    fn test_change() {
        let temp_dir = TempDir::new().unwrap();
        let temp_file = PathBuf::from(temp_dir.path()).join("temp.txt");
        let temp_file_str = temp_file.to_str().unwrap();

        let test_file_seed = seed_test_file(&temp_file_str, Vec::new());

        // Check that simply changing the time correctly, and only, alters the currently running activity
        super::start(&temp_file_str, "test_proj", "test_desc", None).unwrap();

        let target_time = NaiveDateTime::new(
            NaiveDate::from_isoywd_opt(2026, 10, chrono::Weekday::Mon).unwrap(),
            NaiveTime::from_str("10:00").unwrap(),
        );

        let mut original_file_contents = get_file_content(&temp_file_str).unwrap();
        original_file_contents.pop();

        super::change(temp_file_str, None, None, Some(target_time)).unwrap();

        let mut file_contents = get_file_content(&temp_file_str).unwrap();

        let changed_activity = file_contents.pop().unwrap().activity.unwrap();

        assert_eq!(&changed_activity.start, &target_time);
        assert_eq!(test_file_seed, original_file_contents);
    }

    #[test]
    fn test_change_whilst_affecting_prev() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = PathBuf::from(temp_dir.path()).join("temp_file.txt");
        let temp_path_str = temp_path.to_str().unwrap();

        let start_time = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 01, 02).unwrap(),
            NaiveTime::from_num_seconds_from_midnight_opt(3600, 0).unwrap(),
        );
        let initial_finish = start_time + Duration::new(3600, 0);
        let final_finish = start_time + Duration::new(7200, 0);

        // write a project for the previous day which starts at the same time
        // this should not be affected
        let additional_seed_activities = vec![(
            "yesterdays proj",
            "",
            Some(NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2026, 01, 01).unwrap(),
                NaiveTime::from_num_seconds_from_midnight_opt(1200, 0).unwrap(),
            )),
            Some(NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2026, 01, 01).unwrap(),
                NaiveTime::from_num_seconds_from_midnight_opt(3600, 0).unwrap(),
            )),
        )];

        let file_seed = seed_test_file(&temp_path_str, additional_seed_activities);

        super::start(
            &temp_path_str,
            "prev_proj",
            "prev_proj_desc",
            Some(start_time),
        )
        .unwrap();

        super::start(
            &temp_path_str,
            "second_proj",
            "second proj desc",
            Some(initial_finish),
        )
        .unwrap();

        super::change(&temp_path_str, None, None, Some(final_finish)).unwrap();

        let mut file_contents = get_file_content(&temp_path_str).unwrap();
        let test_activities: Vec<Line> = file_contents.split_off(file_contents.len() - 2);

        println!("{test_activities:?}");

        assert_eq!(
            test_activities
                .get(0)
                .unwrap()
                .activity
                .as_ref()
                .unwrap()
                .end
                .unwrap(),
            final_finish
        );

        assert_eq!(
            test_activities
                .get(1)
                .unwrap()
                .activity
                .as_ref()
                .unwrap()
                .start,
            final_finish
        );

        assert_eq!(file_contents, file_seed);
    }
}
