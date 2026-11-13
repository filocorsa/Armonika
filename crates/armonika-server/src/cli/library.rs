//! The `library` command: add, list, rename, move and remove libraries.
//!
//! Libraries are named on the command line. No command touches the files of a library:
//! `remove` deletes the library from the database only.

use std::path::PathBuf;

use armonika_core::db::{Connection, Db};
use armonika_core::libraries::{self, Library};
use clap::Subcommand;

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

/// Runs a `library` subcommand on the database `db`; `data_dir` is the data directory,
/// which no library may overlap.
pub fn run(db: &Db, data_dir: &std::path::Path, command: LibraryCommand) -> anyhow::Result<()> {
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
