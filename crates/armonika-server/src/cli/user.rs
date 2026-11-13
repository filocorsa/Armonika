//! The `user` command: accounts, passwords, flags, library access and API keys.
//!
//! Passwords are read from standard input (`--password-stdin`) or asked on the
//! terminal without echo; they are never command-line arguments and never printed.
//! The only secrets printed are the Subsonic password by `show-subsonic-password` and
//! a new API key, once, by `api-key create`.

use std::io::BufRead;

use anyhow::{Context, bail};
use armonika_core::access::{self, Permission};
use armonika_core::db::{Connection, Db};
use armonika_core::secret::SecretBox;
use armonika_core::users::{self, NewUser, User, UserFlagsUpdate};
use clap::{ArgAction, Args, Subcommand};

use crate::cli::library::find as find_library;

/// Manage users.
#[derive(Debug, Subcommand)]
pub enum UserCommand {
    /// Create a user. The password is also the initial Subsonic password.
    Create {
        /// Login name.
        name: String,
        /// Make the user an admin.
        #[arg(long)]
        admin: bool,
        /// Do not allow downloads of original files.
        #[arg(long)]
        no_download: bool,
        #[command(flatten)]
        password: PasswordSource,
    },
    /// List the users.
    List,
    /// Delete a user and everything that belongs to it.
    Delete {
        /// Login name.
        name: String,
    },
    /// Change the login password. The Subsonic password is not changed.
    SetPassword {
        /// Login name.
        name: String,
        #[command(flatten)]
        password: PasswordSource,
    },
    /// Change the password used by Subsonic apps. The login password is not changed.
    SetSubsonicPassword {
        /// Login name.
        name: String,
        #[command(flatten)]
        password: PasswordSource,
    },
    /// Print the password used by Subsonic apps.
    ShowSubsonicPassword {
        /// Login name.
        name: String,
    },
    /// Change the flags of a user.
    Set {
        /// Login name.
        name: String,
        /// Whether the user is an admin.
        #[arg(long, action = ArgAction::Set, value_name = "true|false")]
        admin: Option<bool>,
        /// Whether the user may download original files.
        #[arg(long, action = ArgAction::Set, value_name = "true|false")]
        download: Option<bool>,
        /// Whether other users can see what this user is playing.
        #[arg(long, action = ArgAction::Set, value_name = "true|false")]
        share_now_playing: Option<bool>,
    },
    /// Give a user access to a library, replacing any previous permission.
    Grant {
        /// Login name.
        name: String,
        /// Name of the library.
        library: String,
        /// `listen` (browse, search and play) or `manage` (also add and fix music).
        permission: Permission,
    },
    /// Take away the access of a user to a library.
    Revoke {
        /// Login name.
        name: String,
        /// Name of the library.
        library: String,
    },
    /// Manage API keys.
    #[command(subcommand)]
    ApiKey(ApiKeyCommand),
}

/// Manage API keys.
#[derive(Debug, Subcommand)]
pub enum ApiKeyCommand {
    /// Create an API key and print it. It cannot be shown again.
    Create {
        /// Login name.
        name: String,
        /// Label of the key, unique for the user (for example the device name).
        label: String,
    },
    /// Delete an API key.
    Revoke {
        /// Login name.
        name: String,
        /// Label of the key.
        label: String,
    },
}

/// Where a new password comes from.
#[derive(Debug, Args)]
pub struct PasswordSource {
    /// Read the password from the first line of standard input instead of asking.
    #[arg(long)]
    password_stdin: bool,
}

impl PasswordSource {
    fn read(&self) -> anyhow::Result<String> {
        if self.password_stdin {
            let mut line = String::new();
            let read = std::io::stdin()
                .lock()
                .read_line(&mut line)
                .context("cannot read the password from standard input")?;
            if read == 0 {
                bail!("no password on standard input");
            }
            let end = line.trim_end_matches(['\r', '\n']).len();
            line.truncate(end);
            return Ok(line);
        }
        let first = rpassword::prompt_password("Password: ")
            .context("cannot read the password from the terminal")?;
        let second = rpassword::prompt_password("Repeat password: ")
            .context("cannot read the password from the terminal")?;
        if first != second {
            bail!("the passwords do not match");
        }
        Ok(first)
    }
}

/// Runs a `user` subcommand on the database `db`.
pub fn run(db: &Db, sb: &SecretBox, command: UserCommand) -> anyhow::Result<()> {
    match command {
        UserCommand::Create {
            name,
            admin,
            no_download,
            password,
        } => {
            let password = password.read()?;
            let user = NewUser {
                name: name.clone(),
                password,
                is_admin: admin,
                can_download: !no_download,
            };
            let id = db.write_blocking(|c| users::create(c, sb, user))?;
            println!("created user {name} ({id})");
        }
        UserCommand::List => {
            let list = db.read_blocking(users::list)?;
            print_users(&list);
        }
        UserCommand::Delete { name } => {
            db.write_blocking(|c| {
                let user = find(c, &name)?;
                users::delete(c, &user.id)
            })?;
            println!("deleted user {name}");
        }
        UserCommand::SetPassword { name, password } => {
            let password = password.read()?;
            db.write_blocking(|c| users::set_password(c, &find(c, &name)?.id, &password))?;
            println!("changed the password of {name}");
        }
        UserCommand::SetSubsonicPassword { name, password } => {
            let password = password.read()?;
            db.write_blocking(|c| {
                users::set_subsonic_password(c, sb, &find(c, &name)?.id, &password)
            })?;
            println!("changed the Subsonic password of {name}");
        }
        UserCommand::ShowSubsonicPassword { name } => {
            let password =
                db.read_blocking(|c| users::subsonic_password(c, sb, &find(c, &name)?.id))?;
            println!("{password}");
        }
        UserCommand::Set {
            name,
            admin,
            download,
            share_now_playing,
        } => {
            let flags = UserFlagsUpdate {
                is_admin: admin,
                can_download: download,
                share_now_playing,
            };
            if flags == UserFlagsUpdate::default() {
                bail!("nothing to change: give --admin, --download or --share-now-playing");
            }
            db.write_blocking(|c| users::set_flags(c, &find(c, &name)?.id, flags))?;
            println!("updated user {name}");
        }
        UserCommand::Grant {
            name,
            library,
            permission,
        } => {
            db.write_blocking(|c| {
                let lib = find_library(c, &library)?;
                access::grant(c, &find(c, &name)?.id, lib.id, permission)
            })?;
            println!("{name} can now {permission} on library {library}");
        }
        UserCommand::Revoke { name, library } => {
            let had = db.write_blocking(|c| {
                let lib = find_library(c, &library)?;
                access::revoke(c, &find(c, &name)?.id, lib.id)
            })?;
            if had {
                println!("revoked the access of {name} to library {library}");
            } else {
                println!("{name} had no access to library {library}");
            }
        }
        UserCommand::ApiKey(ApiKeyCommand::Create { name, label }) => {
            let key =
                db.write_blocking(|c| users::create_api_key(c, &find(c, &name)?.id, &label))?;
            eprintln!("API key {label:?} for {name}; it will not be shown again:");
            println!("{key}");
        }
        UserCommand::ApiKey(ApiKeyCommand::Revoke { name, label }) => {
            db.write_blocking(|c| users::revoke_api_key(c, &find(c, &name)?.id, &label))?;
            println!("revoked API key {label:?} of {name}");
        }
    }
    Ok(())
}

fn find(conn: &Connection, name: &str) -> armonika_core::Result<User> {
    users::by_name(conn, name)?.ok_or_else(|| armonika_core::Error::UserNotFound(name.to_owned()))
}

fn print_users(list: &[User]) {
    if list.is_empty() {
        println!("no users");
        return;
    }
    let width = list
        .iter()
        .map(|u| u.name.chars().count())
        .max()
        .unwrap_or(0);
    for user in list {
        let mut flags = Vec::new();
        if user.is_admin {
            flags.push("admin");
        }
        if !user.can_download {
            flags.push("no-download");
        }
        if user.share_now_playing {
            flags.push("share-now-playing");
        }
        println!("{:<width$}  {}  {}", user.name, user.id, flags.join(","));
    }
}
