// Copyright (c) 2022, Valaphee.
// All rights reserved.

use std::borrow::{Borrow, BorrowMut};
use std::fs::{copy, create_dir, create_dir_all, File, remove_dir_all};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::raw::ino_t;
use std::path::Path;
use std::str::from_utf8;
use aes::Aes256;
use aes::cipher::KeyIvInit;
use cfb8::cipher::AsyncStreamCipher;
use clap::{arg, Arg, ArgAction, Command, Parser};
use clap::builder::Str;
use glob::glob;
use mimalloc::MiMalloc;
use rand::{Rng, thread_rng};
use rand::distributions::Alphanumeric;
use serde::{Deserialize, Serialize};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Parser)]
#[clap(about)]
enum McrpCommand {
    /// Encrypts the folder with a given or auto-generated key
    Encrypt {
        /// Input file or folder
        input: String,
        /// Output folder
        output: String,
        #[clap(short, long)]
        /// Key used for encryption
        key: Option<String>,
        #[clap(short, long)]
        /// Specifies files which should not be encrypted
        exclude: Vec<String>
    },
    /// Decrypts the folder with a given key
    Decrypt {
        /// Input file or folder
        input: String,
        /// Output folder
        output: String,
        #[clap(short, long)]
        /// Key used for decryption
        key: Option<String>,
    }
}

type Aes256Cfb8Enc = cfb8::Encryptor<Aes256>;
type Aes256Cfb8Dec = cfb8::Decryptor<Aes256>;

#[derive(Serialize, Deserialize, Debug)]
struct Content {
    version: u32,
    content: Vec<ContentEntry>
}

#[derive(Serialize, Deserialize, Debug)]
struct ContentEntry {
    path: String,
    key: Option<String>
}

fn main() {
    match McrpCommand::parse() {
        McrpCommand::Encrypt { input, output, key, exclude } => {
            let input_path = Path::new(&input);

            let output_path = Path::new(&output);
            if output_path.exists() {
                remove_dir_all(output_path);
            }
            create_dir_all(output_path);

            let mut content_entries = Vec::new();
            for path in glob(&format!("{}/**/*.*", input)).unwrap() {
                let input_entry_path = path.unwrap();
                let relative_path = input_entry_path.strip_prefix(input_path).unwrap().to_str().unwrap().to_owned();
                let output_entry_path = output_path.join(&relative_path);

                content_entries.push(ContentEntry {
                    key: if exclude.contains(&relative_path) {
                        copy(input_entry_path, output_entry_path);

                        None
                    } else {
                        create_dir_all(output_entry_path.parent().unwrap());

                        let mut file = File::open(input_entry_path).unwrap();
                        let mut buffer = Vec::new();
                        file.read_to_end(&mut buffer);

                        let mut key_buffer = Vec::new();
                        let mut rng = thread_rng();
                        key_buffer.write((0..32).map(|_| rng.sample(Alphanumeric) as char).collect::<String>().as_bytes());
                        Aes256Cfb8Enc::new_from_slices(&key_buffer, &key_buffer[0..16]).unwrap().encrypt(&mut buffer);

                        File::create(output_entry_path).unwrap().write(&buffer);

                        Some(from_utf8(&key_buffer).unwrap().to_owned())
                    },
                    path: relative_path
                })
            }

            let mut file = File::create(output_path.join("contents.json")).unwrap();
            file.write(&[0x00u8, 0x00u8, 0x00u8, 0x00u8, 0xFCu8, 0xB9u8, 0xCFu8, 0x9Bu8]);
            file.seek(SeekFrom::Start(256));
            let content = Content {
                version: 1,
                content: content_entries
            };
            let mut buffer = serde_json::to_vec(&content).unwrap();

            let mut key_buffer = Vec::new();
            let key_bytes = match key {
                None => {
                    let mut rng = thread_rng();
                    key_buffer.write((0..32).map(|_| rng.sample(Alphanumeric) as char).collect::<String>().as_bytes());
                    key_buffer.borrow()
                },
                Some(ref key) => key.as_bytes()
            };
            Aes256Cfb8Enc::new_from_slices(&key_buffer, &key_buffer[0..16]).unwrap().encrypt(&mut buffer);

            file.write(&buffer);
            File::create(format!("{}.key", output)).unwrap().write(key_bytes);
        }
        McrpCommand::Decrypt { input, output, key } => {
            let input_path = Path::new(&input);

            let output_path = Path::new(&output);
            if output_path.exists() {
                remove_dir_all(output_path);
            }
            create_dir_all(output_path);

            let content = {
                let mut file = File::open(input_path.join("contents.json")).unwrap();
                file.seek(SeekFrom::Start(256));
                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer);

                let mut key_buffer = Vec::new();
                let key_bytes = match key {
                    None => {
                        File::open(format!("{}.key", input)).unwrap().read(&mut key_buffer);
                        key_buffer.borrow()
                    },
                    Some(ref key) => key.as_bytes()
                };
                Aes256Cfb8Dec::new_from_slices(&key_bytes, &key_bytes[0..16]).unwrap().decrypt(&mut buffer);

                serde_json::from_slice::<Content>(&buffer).unwrap()
            };

            for content_entry in &content.content {
                let input_entry_path = input_path.join(&content_entry.path);
                let output_entry_path = output_path.join(&content_entry.path);

                match &content_entry.key {
                    None => {
                        copy(input_entry_path, output_entry_path);
                    }
                    Some(key) => {
                        create_dir_all(output_entry_path.parent().unwrap());

                        let mut file = File::open(input_entry_path).unwrap();
                        let mut buffer = Vec::new();
                        file.read_to_end(&mut buffer);

                        let key_bytes = key.as_bytes();
                        Aes256Cfb8Dec::new_from_slices(key_bytes, &key_bytes[0..16]).unwrap().decrypt(&mut buffer);

                        File::create(output_entry_path).unwrap().write(&buffer);
                    }
                }
            }
        }
    }
}
