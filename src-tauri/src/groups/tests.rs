use std::os::unix::fs::symlink;

use rusqlite::params;

use super::*;

struct Fixture {
    data: tempfile::TempDir,
    home: tempfile::TempDir,
    groups: tempfile::TempDir,
    workspaces: tempfile::TempDir,
    projects: tempfile::TempDir,
    db: Database,
}

impl Fixture {
    fn new() -> Self {
        let data = tempfile::tempdir().unwrap();
        let db = Database::open(data.path()).unwrap();
        Self {
            data,
            home: tempfile::tempdir().unwrap(),
            groups: tempfile::tempdir().unwrap(),
            workspaces: tempfile::tempdir().unwrap(),
            projects: tempfile::tempdir().unwrap(),
            db,
        }
    }

    fn config(&self) -> GroupsConfig {
        GroupsConfig {
            root: Ok(self.groups.path().join("groups")),
            home: Some(self.home.path().to_path_buf()),
        }
    }

    fn agent(&self, name: &str) -> String {
        let folder = self.workspaces.path().join(name.to_lowercase());
        fs::create_dir_all(&folder).unwrap();
        self.agent_at(name, &folder)
    }

    fn agent_at(&self, name: &str, folder: &Path) -> String {
        let id = format!("id-{}", name.to_lowercase());
        self.db.with_connection(|c| {
            c.execute(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                 VALUES (?1, ?2, 'blue', 'claude-code', 'default', ?3, 1, 1)",
                params![id, name, folder.to_string_lossy()],
            )
            .unwrap()
        });
        id
    }

    fn delete_agent(&self, id: &str) {
        let root = Ok(self.workspaces.path().to_path_buf());
        crate::agents::remove::delete(&self.db, &root, id, false, &|_| Ok(())).unwrap();
    }

    fn input(
        &self,
        name: &str,
        members: &[&str],
        lead: &str,
        folder: Option<&Path>,
    ) -> CreateGroupInput {
        CreateGroupInput {
            name: name.to_string(),
            folder_path: folder.map(|path| path.to_string_lossy().into_owned()),
            member_ids: members.iter().map(|id| id.to_string()).collect(),
            lead_agent_id: lead.to_string(),
        }
    }

    fn create(&self, input: CreateGroupInput) -> Result<Created, GroupError> {
        create(&self.db, &self.config(), self.data.path(), input)
    }

    fn pair_group(&self, name: &str, folder: Option<&Path>) -> (Group, String, String) {
        let a = self.agent(&format!("{name}-a"));
        let b = self.agent(&format!("{name}-b"));
        let created = self
            .create(self.input(name, &[&a, &b], &a, folder))
            .unwrap();
        (created.group, a, b)
    }

    fn groups(&self) -> Vec<Group> {
        list(&self.db).unwrap()
    }

    fn project(&self, name: &str) -> PathBuf {
        let path = self.projects.path().join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }
}

fn code<T: std::fmt::Debug>(result: Result<T, GroupError>) -> &'static str {
    result.expect_err("expected an error").code
}

fn refusal_message(result: Result<Created, GroupError>) -> String {
    let error = result.expect_err("expected an error");
    assert_eq!(error.code, "folder-refused");
    error.message
}

#[test]
fn a_group_is_created_with_members_in_order_and_a_lead() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let c = fx.agent("Cy");
    let folder = fx.project("repo");
    let created = fx
        .create(fx.input("  Team ", &[&b, &a, &c], &a, Some(&folder)))
        .unwrap();
    assert!(created.shared_folder_with.is_empty());
    let group = created.group;
    assert_eq!(group.name, "Team");
    assert_eq!(group.lead_agent_id.as_deref(), Some(a.as_str()));
    assert_eq!(
        group.folder_path,
        fs::canonicalize(&folder).unwrap().to_string_lossy()
    );
    let names: Vec<&str> = group.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["Bo", "Ada", "Cy"]);
    assert_eq!(group.attention, None);
    assert_eq!(fx.groups(), vec![group]);
}

#[test]
fn each_name_rule_has_its_own_code() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let folder = fx.project("repo");
    let try_name = |name: &str| fx.create(fx.input(name, &[&a, &b], &a, Some(&folder)));
    assert_eq!(code(try_name("  ")), "name-empty");
    assert_eq!(code(try_name(&"a".repeat(41))), "name-too-long");
    assert_eq!(code(try_name("a\nb")), "name-invalid");
    assert!(try_name(&"a".repeat(40)).is_ok());
    assert_eq!(code(try_name(&"A".repeat(40))), "name-taken");
}

#[test]
fn names_are_unique_ignoring_case() {
    let fx = Fixture::new();
    let (_, a, b) = fx.pair_group("Team", None);
    assert_eq!(
        code(fx.create(fx.input(" TEAM ", &[&a, &b], &a, None))),
        "name-taken"
    );
    assert_eq!(fx.groups().len(), 1);
}

#[test]
fn a_group_needs_two_distinct_existing_members_and_a_member_lead() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let c = fx.agent("Cy");
    let folder = fx.project("repo");
    let make =
        |members: &[&str], lead: &str| fx.create(fx.input("Team", members, lead, Some(&folder)));
    assert_eq!(code(make(&[], &a)), "members-too-few");
    assert_eq!(code(make(&[&a], &a)), "members-too-few");
    assert_eq!(code(make(&[&a, &a], &a)), "members-too-few");
    assert_eq!(code(make(&[&a, "missing"], &a)), "member-not-found");
    assert_eq!(code(make(&[&a, &b], &c)), "lead-not-member");
    assert_eq!(code(make(&[&a, &b], "missing")), "lead-not-member");
    assert_eq!(fx.groups().len(), 0);
    assert!(make(&[&a, &b, &a], &b).is_ok());
}

#[test]
fn the_default_folder_is_made_under_the_groups_root_and_numbered() {
    let fx = Fixture::new();
    let (first, a, b) = fx.pair_group("Team Alpha", None);
    let root = fs::canonicalize(fx.groups.path()).unwrap().join("groups");
    assert_eq!(Path::new(&first.folder_path), root.join("team-alpha"));
    assert!(root.join("team-alpha").is_dir());
    let second = fx
        .create(fx.input("team-alpha", &[&a, &b], &a, None))
        .unwrap();
    assert_eq!(
        Path::new(&second.group.folder_path),
        root.join("team-alpha-2")
    );
    assert!(second.shared_folder_with.is_empty());
    let thai = fx.create(fx.input("สวัสดี", &[&a, &b], &a, None)).unwrap();
    assert_eq!(Path::new(&thai.group.folder_path), root.join("group"));
}

#[test]
fn the_suggestion_is_the_next_free_default_path_and_creates_nothing() {
    let fx = Fixture::new();
    let config = fx.config();
    let root = config.root.clone().unwrap();
    assert_eq!(
        suggest(&fx.db, &config, "Team Alpha").unwrap(),
        root.join("team-alpha").to_string_lossy()
    );
    assert!(!root.exists());
    fx.pair_group("Team Alpha", None);
    let suggested = suggest(&fx.db, &fx.config(), "Team Alpha").unwrap();
    assert!(suggested.ends_with("team-alpha-2"), "{suggested}");
    assert!(!Path::new(&suggested).exists());
}

#[test]
fn a_default_folder_that_is_refused_is_removed_and_nothing_else_is() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let workspace = fx.workspaces.path().join("ada");
    fs::write(workspace.join("notes.txt"), "x").unwrap();
    let config = GroupsConfig {
        root: Ok(workspace.join("groups")),
        home: None,
    };
    let result = create(
        &fx.db,
        &config,
        fx.data.path(),
        fx.input("Team", &[&a, &b], &a, None),
    );
    let message = result.expect_err("expected a refusal");
    assert_eq!(message.code, "folder-refused");
    assert!(message.message.contains("inside an agent's workspace"));
    assert!(!workspace.join("groups").join("team").exists());
    assert!(workspace.join("groups").is_dir());
    assert!(workspace.join("notes.txt").is_file());
    assert_eq!(fx.groups().len(), 0);
}

#[test]
fn a_default_folder_is_removed_when_the_group_cannot_be_saved() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let root = fx.config().root.unwrap();
    fs::create_dir_all(root.join("other")).unwrap();
    fx.db.with_connection(|c| {
        c.execute_batch(
            "CREATE TRIGGER refuse_members BEFORE INSERT ON group_members
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap()
    });
    let result = fx.create(fx.input("Team", &[&a, &b], &a, None));
    assert_eq!(code(result), "database-failed");
    assert!(!root.join("team").exists());
    assert!(root.join("other").is_dir());
    assert_eq!(fx.groups().len(), 0);
    assert_eq!(
        code(fx.create(fx.input("Team", &[&a, "missing"], &a, None))),
        "member-not-found"
    );
    assert!(!root.join("team").exists());
    let broken = GroupsConfig {
        root: Err("no root".to_string()),
        home: None,
    };
    let result = create(
        &fx.db,
        &broken,
        fx.data.path(),
        fx.input("Team", &[&a, &b], &a, None),
    );
    assert_eq!(code(result), "folder-failed");
}

#[test]
fn a_relative_or_missing_or_file_folder_is_invalid() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let file = fx.projects.path().join("file.txt");
    fs::write(&file, "x").unwrap();
    for bad in [
        PathBuf::from("relative/dir"),
        fx.projects.path().join("missing"),
        file,
    ] {
        assert_eq!(
            code(fx.create(fx.input("Team", &[&a, &b], &a, Some(&bad)))),
            "folder-invalid",
            "{bad:?}"
        );
    }
}

#[test]
fn a_folder_that_is_or_sits_inside_a_protected_location_is_refused() {
    let fx = Fixture::new();
    let try_folder = |path: &Path| {
        let a = fx.agent(&format!("a{}", path.display().to_string().len()));
        let b = fx.agent(&format!("b{}", path.display().to_string().len()));
        fx.create(fx.input("Team", &[&a, &b], &a, Some(path)))
    };
    let ssh = fx.home.path().join(".ssh");
    fs::create_dir_all(ssh.join("keys")).unwrap();
    let keychains = fx.home.path().join("Library").join("Keychains");
    fs::create_dir_all(&keychains).unwrap();
    for path in [
        ssh.clone(),
        ssh.join("keys"),
        keychains,
        PathBuf::from("/Volumes"),
    ] {
        assert_eq!(
            refusal_message(try_folder(&path)),
            "This folder cannot be used. It is inside a protected location.",
            "{}",
            path.display()
        );
    }
    let through_link = fx.projects.path().join("ssh-link");
    symlink(&ssh, &through_link).unwrap();
    assert!(refusal_message(try_folder(&through_link)).contains("protected location"));
    let near = fx.home.path().join(".ssh-notes");
    fs::create_dir_all(&near).unwrap();
    assert!(try_folder(&near).is_ok());
}

#[test]
fn a_folder_whose_name_has_pattern_characters_is_refused() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    for name in ["a*b", "a?b", "a[b", "a]b"] {
        let folder = fx.project(name);
        assert_eq!(
            refusal_message(fx.create(fx.input("Team", &[&a, &b], &a, Some(&folder)))),
            "This folder cannot be used. Its name contains characters the sandbox cannot use safely.",
            "{name}"
        );
    }
    assert_eq!(fx.groups().len(), 0);
}

#[test]
fn the_disk_root_and_the_home_folder_are_refused() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let try_folder = |path: &Path| fx.create(fx.input("Team", &[&a, &b], &a, Some(path)));
    assert!(refusal_message(try_folder(Path::new("/"))).contains("disk root"));
    assert!(refusal_message(try_folder(fx.home.path())).contains("home folder"));
    let through_link = fx.projects.path().join("home-link");
    symlink(fx.home.path(), &through_link).unwrap();
    assert!(refusal_message(try_folder(&through_link)).contains("home folder"));
    assert_eq!(fx.groups().len(), 0);
}

#[test]
fn the_data_folder_is_refused_with_what_is_inside_it_and_what_contains_it() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let try_folder = |path: &Path| fx.create(fx.input("Team", &[&a, &b], &a, Some(path)));
    assert!(refusal_message(try_folder(fx.data.path())).contains("It is Shikigami's data folder"));
    let inside = fx.data.path().join("inner");
    fs::create_dir(&inside).unwrap();
    assert!(refusal_message(try_folder(&inside)).contains("inside Shikigami's data folder"));
    let parent = fx.data.path().parent().unwrap();
    assert!(refusal_message(try_folder(parent)).contains("contains Shikigami's data folder"));
}

#[test]
fn an_agent_workspace_and_anything_inside_or_around_it_is_refused() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let workspace = fx.workspaces.path().join("ada");
    let inner = workspace.join("src");
    fs::create_dir(&inner).unwrap();
    let try_folder = |path: &Path| fx.create(fx.input("Team", &[&a, &b], &a, Some(path)));
    assert!(refusal_message(try_folder(&workspace)).contains("is an agent's workspace"));
    assert!(refusal_message(try_folder(&inner)).contains("inside an agent's workspace"));
    assert!(
        refusal_message(try_folder(fx.workspaces.path())).contains("contains an agent's workspace")
    );
    assert_eq!(fx.groups().len(), 0);
}

#[test]
fn a_symlink_to_a_workspace_is_refused() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let link = fx.projects.path().join("link");
    symlink(fx.workspaces.path().join("ada"), &link).unwrap();
    assert!(
        refusal_message(fx.create(fx.input("Team", &[&a, &b], &a, Some(&link))))
            .contains("is an agent's workspace")
    );
    let link_to_parent = fx.projects.path().join("parent-link");
    symlink(fx.workspaces.path(), &link_to_parent).unwrap();
    assert!(
        refusal_message(fx.create(fx.input("Team", &[&a, &b], &a, Some(&link_to_parent))))
            .contains("contains an agent's workspace")
    );
}

#[test]
fn a_missing_workspace_behind_a_symlinked_prefix_still_protects_its_path() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let real = fx.project("real");
    let link = fx.projects.path().join("link");
    symlink(&real, &link).unwrap();
    let missing = link.join("gone");
    let b = fx.agent_at("Bo", &missing);
    assert!(!missing.exists());
    let message = refusal_message(fx.create(fx.input("Team", &[&a, &b], &a, Some(&real))));
    assert!(
        message.contains("contains an agent's workspace"),
        "{message}"
    );
    assert!(folder::resolved(&missing).ends_with("real/gone"));
}

#[test]
fn a_firmlinked_alias_of_the_home_folder_is_refused() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let data_volume = Path::new("/System/Volumes/Data");
    assert!(data_volume.is_dir());
    let alias = data_volume.join(
        fs::canonicalize(fx.home.path())
            .unwrap()
            .strip_prefix("/")
            .unwrap(),
    );
    assert!(alias.is_dir());
    assert_ne!(
        fs::canonicalize(&alias).unwrap(),
        fs::canonicalize(fx.home.path()).unwrap()
    );
    let message = refusal_message(fx.create(fx.input("Team", &[&a, &b], &a, Some(&alias))));
    assert!(message.contains("home folder"), "{message}");
}

#[test]
fn firmlinked_aliases_of_the_data_folder_and_a_workspace_are_refused_by_relation() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let alias = |path: &Path| {
        Path::new("/System/Volumes/Data")
            .join(fs::canonicalize(path).unwrap().strip_prefix("/").unwrap())
    };
    let inner = fx.workspaces.path().join("ada").join("src");
    fs::create_dir(&inner).unwrap();
    let try_folder = |path: &Path| fx.create(fx.input("Team", &[&a, &b], &a, Some(path)));
    assert!(refusal_message(try_folder(&alias(fx.data.path())))
        .contains("It is Shikigami's data folder"));
    assert!(refusal_message(try_folder(&alias(&inner))).contains("inside an agent's workspace"));
    assert!(refusal_message(try_folder(&alias(fx.workspaces.path())))
        .contains("contains an agent's workspace"));
    assert!(
        refusal_message(try_folder(&alias(&fx.workspaces.path().join("ada"))))
            .contains("is an agent's workspace")
    );
}

#[test]
fn a_mount_point_counts_as_a_disk_root() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let message = refusal_message(fx.create(fx.input(
        "Team",
        &[&a, &b],
        &a,
        Some(Path::new("/System/Volumes/Data")),
    )));
    assert!(message.contains("root of a disk"), "{message}");
    assert!(fx.groups().is_empty());
}

#[test]
fn a_shared_folder_is_allowed_and_reported_by_group_name() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let folder = fx.project("repo");
    let first = fx
        .create(fx.input("One", &[&a, &b], &a, Some(&folder)))
        .unwrap();
    assert!(first.shared_folder_with.is_empty());
    let link = fx.projects.path().join("repo-link");
    symlink(&folder, &link).unwrap();
    let second = fx
        .create(fx.input("Two", &[&a, &b], &b, Some(&link)))
        .unwrap();
    assert_eq!(second.shared_folder_with, ["One"]);
    assert_eq!(fx.groups().len(), 2);
}

#[test]
fn deleting_a_group_keeps_its_folder_and_files_and_the_agents() {
    let fx = Fixture::new();
    let folder = fx.project("repo");
    fs::write(folder.join("file.txt"), "x").unwrap();
    let (group, _, _) = fx.pair_group("Team", Some(&folder));
    delete(&fx.db, &group.id).unwrap();
    assert!(folder.join("file.txt").is_file());
    assert_eq!(fx.groups().len(), 0);
    let members: i64 = fx
        .db
        .with_connection(|c| c.query_row("SELECT COUNT(*) FROM group_members", [], |r| r.get(0)))
        .unwrap();
    let agents: i64 = fx
        .db
        .with_connection(|c| c.query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0)))
        .unwrap();
    assert_eq!((members, agents), (0, 2));
    assert_eq!(code(delete(&fx.db, &group.id)), "not-found");
}

#[test]
fn queued_messages_of_removed_members_move_to_the_lead_when_members_change() {
    let fx = Fixture::new();
    let folder = fx.project("repo");
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let c = fx.agent("Cy");
    let created = fx
        .create(fx.input("Team", &[&a, &b, &c], &a, Some(&folder)))
        .unwrap();
    let id = created.group.id;
    let queue = |agent: &str, position: i64| {
        fx.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
                 VALUES (?1, ?2, ?3, 'text', ?4, 1)",
                params![format!("q{position}"), id, agent, position],
            )
            .unwrap()
        });
    };
    queue(&c, 0);
    queue(&b, 1);
    queue(&a, 2);
    let addressed = || -> Vec<String> {
        fx.db.with_connection(|conn| {
            conn.prepare("SELECT addressed_agent_id FROM group_queue ORDER BY position")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        })
    };
    update(
        &fx.db,
        &id,
        UpdateGroupInput {
            member_ids: Some(vec![a.clone(), b.clone()]),
            ..UpdateGroupInput::default()
        },
    )
    .unwrap();
    assert_eq!(addressed(), [a.clone(), b.clone(), a.clone()]);
    update(
        &fx.db,
        &id,
        UpdateGroupInput {
            member_ids: Some(vec![b.clone(), c.clone()]),
            lead_agent_id: Some(c.clone()),
            ..UpdateGroupInput::default()
        },
    )
    .unwrap();
    assert_eq!(addressed(), [c.clone(), b.clone(), c.clone()]);
}

#[test]
fn a_group_with_a_running_turn_cannot_be_deleted_until_the_turn_ends() {
    let fx = Fixture::new();
    let folder = fx.project("repo");
    let (group, a, _) = fx.pair_group("Team", Some(&folder));
    fx.db.with_connection(|c| {
        c.execute(
            "INSERT INTO turns (id, agent_id, group_id, status, started_at)
             VALUES ('t', ?1, ?2, 'running', 1)",
            params![a, group.id],
        )
        .unwrap()
    });
    assert_eq!(code(delete(&fx.db, &group.id)), "busy");
    assert_eq!(fx.groups().len(), 1);
    fx.db.with_connection(|c| {
        c.execute("UPDATE turns SET status = 'stopped'", [])
            .unwrap()
    });
    delete(&fx.db, &group.id).unwrap();
    assert_eq!(fx.groups().len(), 0);
}

#[test]
fn a_group_can_be_renamed_and_re_membered_and_given_a_new_lead() {
    let fx = Fixture::new();
    let folder = fx.project("repo");
    let (group, a, b) = fx.pair_group("Team", Some(&folder));
    let c = fx.agent("Cy");
    let renamed = update(
        &fx.db,
        &group.id,
        UpdateGroupInput {
            name: Some("Crew".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(renamed.name, "Crew");
    assert_eq!(renamed.members, group.members);
    let changed = update(
        &fx.db,
        &group.id,
        UpdateGroupInput {
            member_ids: Some(vec![c.clone(), b.clone()]),
            lead_agent_id: Some(c.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let names: Vec<&str> = changed.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["Cy", "Team-b"]);
    assert_eq!(changed.lead_agent_id.as_deref(), Some(c.as_str()));
    assert!(!changed.members.iter().any(|m| m.agent_id == a));
}

#[test]
fn an_update_keeps_the_same_rules() {
    let fx = Fixture::new();
    let (group, a, b) = fx.pair_group("Team", None);
    let (other, _, _) = fx.pair_group("Other", None);
    let c = fx.agent("Cy");
    let try_update = |input: UpdateGroupInput| update(&fx.db, &group.id, input);
    assert_eq!(
        code(try_update(UpdateGroupInput {
            name: Some("other".to_string()),
            ..Default::default()
        })),
        "name-taken"
    );
    assert!(try_update(UpdateGroupInput {
        name: Some("TEAM".to_string()),
        ..Default::default()
    })
    .is_ok());
    assert_eq!(
        code(try_update(UpdateGroupInput {
            member_ids: Some(vec![a.clone()]),
            ..Default::default()
        })),
        "members-too-few"
    );
    assert_eq!(
        code(try_update(UpdateGroupInput {
            member_ids: Some(vec![b.clone(), c.clone()]),
            ..Default::default()
        })),
        "lead-not-member"
    );
    assert_eq!(
        code(try_update(UpdateGroupInput {
            lead_agent_id: Some(c.clone()),
            ..Default::default()
        })),
        "lead-not-member"
    );
    assert_eq!(
        code(update(&fx.db, "missing", UpdateGroupInput::default())),
        "not-found"
    );
    assert_eq!(fx.groups().iter().filter(|g| g.id == other.id).count(), 1);
}

#[test]
fn a_refused_update_changes_nothing() {
    let fx = Fixture::new();
    let (group, a, b) = fx.pair_group("Team", None);
    let c = fx.agent("Cy");
    let result = update(
        &fx.db,
        &group.id,
        UpdateGroupInput {
            name: Some("Renamed".to_string()),
            member_ids: Some(vec![b.clone(), c.clone()]),
            lead_agent_id: Some(a.clone()),
        },
    );
    assert_eq!(code(result), "lead-not-member");
    let after = fx.groups().remove(0);
    assert_eq!(after.name, "Team");
    assert_eq!(after.members, group.members);
}

#[test]
fn deleting_an_agent_reassigns_the_lead_and_leaves_too_few_members() {
    let fx = Fixture::new();
    let folder = fx.project("repo");
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let c = fx.agent("Cy");
    let created = fx
        .create(fx.input("Team", &[&a, &b, &c], &a, Some(&folder)))
        .unwrap();
    let other = fx
        .create(fx.input("Other", &[&b, &c], &c, Some(&folder)))
        .unwrap();
    fx.delete_agent(&a);
    let group = fx
        .groups()
        .into_iter()
        .find(|g| g.id == created.group.id)
        .unwrap();
    assert_eq!(group.lead_agent_id.as_deref(), Some(b.as_str()));
    assert_eq!(group.members.len(), 2);
    assert_eq!(group.attention, None);
    let untouched = fx
        .groups()
        .into_iter()
        .find(|g| g.id == other.group.id)
        .unwrap();
    assert_eq!(untouched.lead_agent_id.as_deref(), Some(c.as_str()));
    fx.delete_agent(&b);
    let group = fx
        .groups()
        .into_iter()
        .find(|g| g.id == created.group.id)
        .unwrap();
    assert_eq!(group.lead_agent_id.as_deref(), Some(c.as_str()));
    assert_eq!(group.members.len(), 1);
    let attention = group.attention.unwrap();
    assert_eq!(attention.code, "too-few-members");
    assert_eq!(attention.message, "Add another agent to use this group.");
    fx.delete_agent(&c);
    let group = fx
        .groups()
        .into_iter()
        .find(|g| g.id == created.group.id)
        .unwrap();
    assert_eq!(group.lead_agent_id, None);
    assert!(group.members.is_empty());
    assert!(folder.is_dir());
}

#[test]
fn the_lead_moves_to_the_first_remaining_member_by_position() {
    let fx = Fixture::new();
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let c = fx.agent("Cy");
    let d = fx.agent("Di");
    let created = fx
        .create(fx.input("Team", &[&d, &a, &c, &b], &a, None))
        .unwrap();
    fx.delete_agent(&a);
    let group = fx.groups().remove(0);
    assert_eq!(group.id, created.group.id);
    assert_eq!(group.lead_agent_id.as_deref(), Some(d.as_str()));
}

#[test]
fn a_missing_folder_needs_attention_and_the_group_stays_listed() {
    let fx = Fixture::new();
    let folder = fx.project("repo");
    let (group, _, _) = fx.pair_group("Team", Some(&folder));
    assert_eq!(group.attention, None);
    fs::remove_dir(&folder).unwrap();
    let attention = fx.groups().remove(0).attention.unwrap();
    assert_eq!(attention.code, "folder-missing");
    assert_eq!(attention.message, "The group folder is missing.");
}

#[test]
fn groups_are_listed_by_latest_activity_then_newest() {
    let fx = Fixture::new();
    let (first, _, _) = fx.pair_group("First", None);
    let (second, _, _) = fx.pair_group("Second", None);
    let (third, _, _) = fx.pair_group("Third", None);
    fx.db.with_connection(|c| {
        c.execute(
            "UPDATE groups SET last_activity_at = 100, created_at = 1 WHERE id = ?1",
            [&first.id],
        )
        .unwrap();
        c.execute(
            "UPDATE groups SET last_activity_at = 100, created_at = 2 WHERE id = ?1",
            [&second.id],
        )
        .unwrap();
        c.execute(
            "UPDATE groups SET last_activity_at = 50 WHERE id = ?1",
            [&third.id],
        )
        .unwrap();
    });
    let order: Vec<String> = fx.groups().into_iter().map(|g| g.name).collect();
    assert_eq!(order, ["Second", "First", "Third"]);
}

#[test]
fn the_groups_root_defaults_to_home_and_rejects_a_relative_override() {
    let home = Path::new("/Users/me");
    assert_eq!(
        resolve_groups_root(Some(home), None).unwrap(),
        PathBuf::from("/Users/me/Shikigami/groups")
    );
    assert_eq!(
        resolve_groups_root(Some(home), Some(OsString::from("/tmp/g"))).unwrap(),
        PathBuf::from("/tmp/g")
    );
    assert!(resolve_groups_root(Some(home), Some(OsString::from("rel"))).is_err());
    assert!(resolve_groups_root(None, None).is_err());
}

fn open_hand_off(fx: &Fixture, group: &str, from: &str, to: &str) {
    fx.db.with_connection(|conn| {
        conn.execute(
            "INSERT INTO turns (id, agent_id, group_id, status, started_at, root_message_id, deferred)
             VALUES ('t1', ?1, ?2, 'completed', 1, 'r', 1)",
            params![from, group],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, kind, root_message_id, parent_turn_id, position, created_at)
             VALUES ('q', ?1, ?2, 'task', 'hand-off', 'r', 't1', 0, 1)",
            params![group, to],
        )
        .unwrap();
    });
}

fn queue_rows(fx: &Fixture) -> Vec<(String, String, String)> {
    fx.db.with_connection(|conn| {
        conn.prepare("SELECT addressed_agent_id, kind, content FROM group_queue ORDER BY position")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    })
}

#[test]
fn a_member_removed_from_a_group_resolves_the_debt_of_its_waiting_hand_off() {
    let fx = Fixture::new();
    let folder = fx.project("repo");
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let c = fx.agent("Cy");
    let created = fx
        .create(fx.input("Team", &[&a, &b, &c], &a, Some(&folder)))
        .unwrap();
    let id = created.group.id;
    open_hand_off(&fx, &id, &a, &b);
    update(
        &fx.db,
        &id,
        UpdateGroupInput {
            member_ids: Some(vec![a.clone(), c.clone()]),
            ..UpdateGroupInput::default()
        },
    )
    .unwrap();
    assert_eq!(
        queue_rows(&fx),
        [(
            a,
            "continuation".to_owned(),
            "Continue without Bo's answer.".to_owned()
        )]
    );
}

#[test]
fn a_deleted_agent_resolves_the_debt_of_its_waiting_hand_off() {
    let fx = Fixture::new();
    let folder = fx.project("repo");
    let a = fx.agent("Ada");
    let b = fx.agent("Bo");
    let c = fx.agent("Cy");
    let created = fx
        .create(fx.input("Team", &[&a, &b, &c], &a, Some(&folder)))
        .unwrap();
    open_hand_off(&fx, &created.group.id, &a, &b);
    fx.delete_agent(&b);
    assert_eq!(
        queue_rows(&fx),
        [(
            a,
            "continuation".to_owned(),
            "Continue without Bo's answer.".to_owned()
        )]
    );
}
