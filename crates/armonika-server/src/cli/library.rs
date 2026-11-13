//! The `library` command: add, list, rename, move and remove libraries.
//!
//! Libraries are named on the command line. No command touches the files of a library:
//! `remove` deletes the library from the database only.

use std::path::PathBuf;

use armonika_core::db::{Connection, Db};
use armonika_core::libraries::{self, Library};
use clap::Subcommand;

use crate::config::Config;

/// Manage libraries.
#[derive(Debug, Subcommand)]
pub enum LibraryCommand {
    /// Add a library. Its directory must not overlap another library or the data
    /// directory.
    Add {
        /// Name of the library.
        name: String,
        /// Root directory of the library.
        path: PathBuf,
    },
    /// List the libraries.
    List,
    /// Rename a library.
    Rename {
        /// Current name.
        old: String,
        /// New name.
        new: String,
    },
    /// Move a library to another directory, keeping its identifier and its data.
    SetPath {
        /// Name of the library.
        name: String,
        /// New root directory.
        path: PathBuf,
    },
    /// Remove a library and its tracks from the database. Files on disk are not touched.
    /// Without --yes, only print what would be removed.
    Remove {
        /// Name of the library.
        name: String,
        /// Really remove it.
        #[arg(long)]
        yes: bool,
    },
}

/// Runs a `library` subcommand on the database `db` with the configuration `config`
/// (its data directory, which no library may overlap, and its configured libraries).
pub fn run(db: &Db, config: &Config, command: LibraryCommand) -> anyhow::Result<()> {
    let data_dir = config.data_dir.as_path();
    match command {
        LibraryCommand::Add { name, path } => {
            let id = db.write_blocking(|c| libraries::create(c, &name, &path, data_dir))?;
            println!("added library {name} ({id})");
        }
        LibraryCommand::List => {
            let list = db.read_blocking(libraries::list)?;
            print_libraries(&list);
        }
        LibraryCommand::Rename { old, new } => {
            db.write_blocking(|c| libraries::rename(c, find(c, &old)?.id, &new))?;
            println!("renamed library {old} to {new}");
            hint_if_configured(config, &old);
        }
        LibraryCommand::SetPath { name, path } => {
            let root = db.write_blocking(|c| {
                let id = find(c, &name)?.id;
                libraries::set_root(c, id, &path, data_dir)?;
                Ok(find(c, &name)?.root)
            })?;
            println!("library {name} is now at {}", root.display());
        }
        LibraryCommand::Remove { name, yes } => {
            if yes {
                let summary = db.write_blocking(|c| libraries::remove(c, find(c, &name)?.id))?;
                println!(
                    "removed library {name}: {} tracks deleted from the database, {} playlists \
                     changed; no file was touched",
                    summary.tracks, summary.playlists_touched
                );
                hint_if_configured(config, &name);
            } else {
                let summary =
                    db.read_blocking(|c| libraries::removal_summary(c, find(c, &name)?.id))?;
                println!(
                    "removing library {name} would delete {} tracks from the database and \
                     change {} playlists; files on disk are not touched",
                    summary.tracks, summary.playlists_touched
                );
                println!("nothing done: run again with --yes to remove it");
            }
        }
    }
    Ok(())
}

/// After a rename or a removal: `serve` creates every configured library that is not in
/// the database, so a library still configured under its old name comes back at the next
/// start. Names are compared without ASCII case, like in the database.
fn hint_if_configured(config: &Config, name: &str) {
    if config
        .libraries
        .iter()
        .any(|l| l.name.eq_ignore_ascii_case(name))
    {
        eprintln!(
            "note: the library {name:?} is in the configuration: also remove it from \
             armonika.toml (or the ARMONIKA_ environment), otherwise it will be created again \
             at the next start"
        );
    }
}

/// The library called `name`, or [`armonika_core::Error::LibraryNameNotFound`].
pub fn find(conn: &Connection, name: &str) -> armonika_core::Result<Library> {
    libraries::by_name(conn, name)?
        .ok_or_else(|| armonika_core::Error::LibraryNameNotFound(name.to_owned()))
}

fn print_libraries(list: &[Library]) {
    if list.is_empty() {
        println!("no libraries");
        return;
    }
    let width = list
        .iter()
        .map(|l| l.name.chars().count())
        .max()
        .unwrap_or(0);
    for lib in list {
        println!("{}  {:<width$}  {}", lib.id, lib.name, lib.root.display());
    }
}
