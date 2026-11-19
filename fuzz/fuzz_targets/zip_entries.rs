//! Builds a ZIP from generated entries with the zip writer, then checks that
//! admission accepts exactly the archives without unsafe names, symlinks, or
//! ASCII case aliases, and that it returns every file's content unchanged.
#![no_main]

use arbitrary::Arbitrary;
use bedrock_foundry::{archive::read_zip, storage::safe_relative};
use libfuzzer_sys::fuzz_target;
use std::{
    collections::{BTreeMap, HashSet},
    io::{Cursor, Read, Write},
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

#[derive(Arbitrary, Debug)]
enum Kind {
    File { content: Vec<u8>, deflate: bool },
    Directory,
    Symlink { target: String },
}

#[derive(Arbitrary, Debug)]
struct Entry {
    name: String,
    kind: Kind,
}

fn build(entries: &[Entry]) -> zip::result::ZipResult<Vec<u8>> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for entry in entries {
        match &entry.kind {
            Kind::File { content, deflate } => {
                let method = if *deflate {
                    CompressionMethod::Deflated
                } else {
                    CompressionMethod::Stored
                };
                let options = SimpleFileOptions::default().compression_method(method);
                writer.start_file(entry.name.as_str(), options)?;
                writer.write_all(content)?;
            }
            Kind::Directory => {
                writer.add_directory(entry.name.as_str(), SimpleFileOptions::default())?;
            }
            Kind::Symlink { target } => {
                writer.add_symlink(
                    entry.name.as_str(),
                    target.as_str(),
                    SimpleFileOptions::default(),
                )?;
            }
        }
    }
    Ok(writer.finish()?.into_inner())
}

fuzz_target!(|entries: Vec<Entry>| {
    // The writer rejects some names, such as exact duplicates. Those archives
    // are covered by the zip_admission target.
    let Ok(bytes) = build(&entries) else {
        return;
    };

    // Read the stored entries back with the zip reader to learn the final names.
    let mut archive = ZipArchive::new(Cursor::new(&bytes)).expect("reopen written archive");
    let mut admissible = true;
    let mut folded = HashSet::new();
    let mut expected = BTreeMap::new();
    for index in 0..archive.len() {
        let mut member = archive.by_index(index).expect("read written member");
        let name = member.name().to_owned();
        let symlink = member
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000);
        admissible &= !symlink
            && safe_relative(name.trim_end_matches('/')).is_ok()
            && folded.insert(name.to_ascii_lowercase());
        if !member.is_dir() {
            let mut content = Vec::new();
            member
                .read_to_end(&mut content)
                .expect("read written content");
            expected.insert(name, content);
        }
    }
    // A writer that kept fewer entries than it was given merged duplicates.
    admissible &= archive.len() == entries.len();

    match read_zip(&bytes) {
        Ok(files) => {
            assert!(
                admissible,
                "accepted an archive with an unsafe member: {entries:?}"
            );
            assert_eq!(files, expected, "admission changed member contents");
        }
        Err(error) => assert!(
            !admissible,
            "rejected a safe archive: {error:#}: {entries:?}"
        ),
    }
});
