use chrono::Utc;
use getopts::{Options, Matches};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryInto;
use std::fs;
use std::fs::{File, metadata};
use std::io::Read;
use std::{env, process};
use rand::random;

#[derive(Deserialize)]
struct DBNoteModelField {
    name: String,
    ord: u64,
}

#[derive(Deserialize)]
struct DBNoteModel {
    did: i64,
    flds: Vec<DBNoteModelField>,
    id: i64,
    name: String,
}

#[derive(Deserialize)]
struct DBDeck {
    id: i64,
    name: String,
}

struct NoteModel {
    id: i64,
    fields: usize,
    deck_id: i64,
    name: String,
    note_count: i64,
}

struct Deck {
    id: i64,
    name: String,
    card_count: i64,
}

#[derive(Serialize, Deserialize)]
struct Configuration {
    database_path: Option<String>,
    deck_name: Option<String>,
    model_name: Option<String>,

    #[serde(skip_serializing, default)]
    dirty: bool
}

fn print_usage(program_name: &str, opts: &Options) {
    let brief = format!("Usage: {} [OPTIONS] COMMAND [COMMAND-ARGS...]", program_name);
    print!("{}", opts.usage(&brief));

    println!("Available commands are:");
    println!("    add        Add a new card to the database");
}

fn get_config_path(opts: &Matches) -> String {
    let config_path = match opts.opt_str("c") {
        Some(config_path_str) => config_path_str,
        None => {
            let mut config_file_path = dirs::config_dir().unwrap();
            config_file_path.push("cranki.json");
            String::from(config_file_path.to_str().unwrap())
        }
    };
    config_path
}

fn parse_configuration(config_path: &str, opts: &Matches) -> Configuration {
    let config_loaded = match File::open(config_path) {
        Ok(mut file) => {
            println!("Found config file @ {}", config_path);

            let mut config_json = String::new();
            match file.read_to_string(&mut config_json) {
                Ok(_) => match serde_json::from_str::<Configuration>(&config_json) {
                    Ok(config) => Some(config),
                    Err(e) => {
                        eprintln!("Found configuration file at {}, but failed to parse it: {}", config_path, e);
                        None
                    },
                },
                Err(e) => {
                    eprintln!("Found configuration file at {}, but failed to load it: {}", config_path, e);
                    None
                }
            }
        },
        Err(e) => {
            eprintln!("Failed to open configuration file at {}: {}", config_path, e);
            None
        },
    };

    let config_loaded = match config_loaded {
        Some(config) => config,
        None => Configuration {
            database_path: None,
            deck_name: None,
            model_name: None,
            dirty: false
        },
    };

    let mut dirty = false;
    let mut config = Configuration {
        database_path: match opts.opt_str("f") {
            Some(path) => {
                if let Some(dpath) = config_loaded.database_path {
                    if dpath != path {
                        dirty = true;
                    }
                } else {
                    dirty = true;
                }
                Some(path)
            },
            None => config_loaded.database_path,
        },
        deck_name: match opts.opt_str("d") {
            Some(name) => {
                if let Some(dname) = config_loaded.deck_name {
                    if dname != name {
                        dirty = true;
                    }
                } else {
                    dirty = true;
                }
                Some(name)
            },
            None => config_loaded.deck_name,
        },
        model_name: match opts.opt_str("m") {
            Some(name) => {
                if let Some(mname) = config_loaded.model_name {
                    if mname != name {
                        dirty = true;
                    }
                } else {
                    dirty = true;
                }
                Some(name)
            },
            None => config_loaded.model_name,
        },
        dirty: false,
    };
    config.dirty = dirty;
    config
}

fn write_configuration(config_path: &str, config: &Configuration) {
    let config_json = serde_json::to_string(config).unwrap();
    match fs::write(config_path, config_json) {
        Ok(_) => {
            println!("Configuration successfully written to '{}'", config_path);
        },
        Err(e) => {
            eprintln!("Failed to write configuration to '{}': {}", config_path, e);
        }
    }
}

fn extract_db_info(sql: &sqlite::Connection) -> (Vec::<NoteModel>, Vec::<Deck>, Vec::<String>) {
    // NOTE: We use the database structure as defined at:
    //       https://github.com/ankidroid/Anki-Android/wiki/Database-Structure

    let mut models = Vec::<NoteModel>::new();
    let mut decks = Vec::<Deck>::new();
    let mut notes = Vec::<String>::new();

    let mut col_state = match sql.prepare("SELECT mod, usn, models, decks FROM col") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to extract the required state from the database: {}", e);
            process::exit(1);
        }
    };
    while col_state.next().unwrap() != sqlite::State::Done {
        // println!("mod = {}", col_state.read::<i64>(0).unwrap());
        // println!("usn = {}", col_state.read::<i64>(1).unwrap());
        // TODO: Do we care/need to update the collection's 'mod' column when we add a card?
        // TODO: Do we need to update the collection's usn if its positive and we add a card?

        let db_models_json = col_state.read::<String>(2).unwrap();
        let db_models: HashMap<u64, DBNoteModel> = serde_json::from_str(&db_models_json).unwrap();
        for (_, model) in db_models {
            models.push(NoteModel{
                id: model.id,
                fields: model.flds.len(),
                deck_id: model.did,
                name: model.name,
                note_count: 0,
            });
        }

        let db_decks_json = col_state.read::<String>(3).unwrap();
        let db_decks: HashMap<u64, DBDeck> = serde_json::from_str(&db_decks_json).unwrap();
        for (_, deck) in db_decks {
            decks.push(Deck{
                id: deck.id,
                name: deck.name,
                card_count: 0
            });
        }
    }

    let mut card_stmt = match sql.prepare("SELECT did, COUNT(*) AS count FROM cards GROUP BY did") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to extract the required state from the database: {}", e);
            process::exit(1);
        }
    };
    while card_stmt.next().unwrap() != sqlite::State::Done {
        let deck_id = card_stmt.read::<i64>(0).unwrap();
        let card_count = card_stmt.read::<i64>(1).unwrap();

        for deck in decks.iter_mut() {
            if deck.id == deck_id {
                deck.card_count += card_count;
                break;
            }
        }
    }

    let mut note_stmt = match sql.prepare("SELECT mid, COUNT(*) AS count FROM notes GROUP BY mid") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to extract the required state from the database: {}", e);
            process::exit(1);
        }
    };
    while note_stmt.next().unwrap() != sqlite::State::Done {
        let model_id = note_stmt.read::<i64>(0).unwrap();
        let note_count = note_stmt.read::<i64>(1).unwrap();

        for model in models.iter_mut() {
            if model.id == model_id {
                model.note_count += note_count;
                break;
            }
        }
    }

    let mut note_stmt = match sql.prepare("SELECT flds FROM notes") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to extract the required state from the database: {}", e);
            process::exit(1);
        }
    };
    while note_stmt.next().unwrap() != sqlite::State::Done {
        let fld_str = note_stmt.read::<String>(0).unwrap();
        notes.push(fld_str);
    }

    return (models, decks, notes);
}

fn write_new_entry_to_db(sql: &sqlite::Connection, command_args: &[String], model_id: i64, deck_id: i64, existing_notes: &Vec<String>) {
    let timestamp = Utc::now();
    let timestamp_sec = timestamp.timestamp();
    let timestamp_millis = timestamp.timestamp_millis();
    let uuid = format!("{:x}", random::<u64>()); // TODO: Verify that this doesn't collide with any existing guids
    let sort_field = &command_args[0];
    let fields = command_args.join("\u{1f}");
    // TODO: Check for duplicates with existing notes when one is added

    let sha1_bytes = sha1::Sha1::from(&command_args[0]).digest().bytes();
    let first_field_sha: i64 = u32::from_be_bytes(sha1_bytes[0..4].try_into().unwrap()).into();

    let mut note_insert = sql.prepare(
        "INSERT INTO notes(id, guid, mid, mod, usn, tags, flds, sfld, csum, flags, data)
        VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)").unwrap();
    note_insert.bind( 1, timestamp_millis).unwrap(); // id
    note_insert.bind( 2, uuid.as_str()).unwrap(); // guid
    note_insert.bind( 3, model_id).unwrap(); // mid
    note_insert.bind( 4, timestamp_sec).unwrap(); // mod
    note_insert.bind( 5, -1).unwrap(); // usn
    note_insert.bind( 6, "").unwrap(); // tags
    note_insert.bind( 7, fields.as_str()).unwrap(); // flds
    note_insert.bind( 8, sort_field.as_str()).unwrap(); // sfld
    note_insert.bind( 9, first_field_sha).unwrap(); // csum
    note_insert.bind(10, 0).unwrap(); // flags
    note_insert.bind(11, "").unwrap(); // data
    while note_insert.next().unwrap() != sqlite::State::Done {}

    // TODO: Get the ord value - the index of the template to use for display
    let mut card_insert = sql.prepare(
        "INSERT INTO cards(id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, reps, lapses, left, odue, odid, flags, data)
        VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)").unwrap();
    card_insert.bind( 1, timestamp_millis).unwrap(); // id
    card_insert.bind( 2, timestamp_millis).unwrap(); // nid
    card_insert.bind( 3, deck_id).unwrap(); // did
    card_insert.bind( 4, 0).unwrap(); // TODO: ord
    card_insert.bind( 5, timestamp_sec).unwrap(); // mod
    card_insert.bind( 6, -1).unwrap(); // usn
    card_insert.bind( 7, 0).unwrap(); // type
    card_insert.bind( 8, 0).unwrap(); // queue
    card_insert.bind( 9, timestamp_millis).unwrap(); // due
    card_insert.bind(10, 0).unwrap(); // ivl
    card_insert.bind(11, 0).unwrap(); // factor
    card_insert.bind(12, 0).unwrap(); // reps
    card_insert.bind(13, 0).unwrap(); // lapses
    card_insert.bind(14, 0).unwrap(); // left
    card_insert.bind(15, 0).unwrap(); // odue
    card_insert.bind(16, 0).unwrap(); // odid
    card_insert.bind(17, 0).unwrap(); // flags
    card_insert.bind(18, 0).unwrap(); // data
    while card_insert.next().unwrap() != sqlite::State::Done {}

    println!("New entry successfully added to the database");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let program = &args[0];
    let mut opts_spec = Options::new();
    opts_spec.optflag("h", "help", "Print this help menu");
    opts_spec.optflag("n", "no-store-config", "Don't write a config file (a config file will be written if this is not provided)");
    opts_spec.optopt("f", "database-file", "The path to the anki database (usually with the *.anki2 extension). Overwrites the stored value in the config file", "DATABASE-PATH");
    opts_spec.optopt("d", "deck", "The name of the deck to modify. Overwrites the stored value in the config file", "DECK-NAME");
    opts_spec.optopt("m", "model", "The name of the model to use (if adding a new card). Overwrites the stored value in the config file", "MODEL-NAME");
    opts_spec.optopt("c", "config", "The config file path to use", "CONFIG-PATH");

    let opts = match opts_spec.parse(&args[1..]) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{}", e.to_string());
            eprintln!("Try '{} --help' for more information", program);
            process::exit(1);
        }
    };

    if opts.opt_present("h") {
        println!("crAnki: A simple command-line tool for interacting with Anki database files");
        println!("");

        print_usage(program, &opts_spec);

        println!("");
        println!("For other Anki-related software, see the official website @ https://apps.ankiweb.net/");
        return;
    }

    let config_path = get_config_path(&opts);
    let config = parse_configuration(&config_path, &opts);
    let should_store_config = config.dirty && !opts.opt_present("n");

    let database_path = match &config.database_path {
        Some(path) => {
            match metadata(&path) {
                Ok(md) => {
                    if !md.is_file() {
                        eprintln!("The provided database path ({}) does not point to a standard file", &path);
                        process::exit(1);
                    }
                    String::from(path)
                },
                Err(e) => {
                    eprintln!("Failed the check for a database at the provided path ({}): {}", path, e);
                    process::exit(1);
                }
            }
        },
        None => {
            eprintln!("Database path was not provided as an argument and could not be loaded from the config file");
            process::exit(1);
        }
    };

    let sql = match sqlite::open(&database_path) {
        Ok(connection) => connection,
        Err(e) => {
            eprintln!("Failed to open database file at path {}: {}", &database_path, e);
            process::exit(1);
        }
    };
    let (models, decks, notes) = extract_db_info(&sql);

    let mut input_deck: Option<&Deck> = None;
    match &config.deck_name {
        Some(name) => {
            let mut found = false;
            for d in decks.iter() {
                if &d.name == name {
                    input_deck = Some(d);
                    found = true;
                    break;
                }
            }
            if !found {
                eprintln!("The provided deck name '{}' was not found in the database.", name);
            }
        },
        None => {
            eprintln!("Deck name was not provided as an argument and could not be loaded from the config file");
        }
    };
    let input_deck = match input_deck {
        Some(d) => d,
        None => {
            if decks.len() > 0 {
                eprintln!("Valid deck names are: ");
                for d in decks.iter() {
                    eprintln!("    \"{}\"    ({} existing cards)", &d.name, d.card_count);
                }
            } else {
                eprintln!("The database contains no decks!")
            }
            process::exit(1);
        }
    };

    let mut input_model: Option<&NoteModel> = None;
    let model_name = match &config.model_name {
        Some(name) => {
            for m in models.iter() {
                if &m.name == name {
                    input_model = Some(m);
                    break;
                }
            }
            match input_model {
                Some(m) => String::from(&m.name),
                None => {
                    eprintln!("The provided model name '{}' was not found in the database.", name);
                    String::new()
                }
            }
        },
        None => {
            eprintln!("Model name was not provided as an argument and could not be loaded from the config file");
            String::new()
        }
    };
    let input_model = match input_model {
        Some(m) => m,
        None => {
            if models.len() > 0 {
                eprintln!("Valid model names are: ");
                for m in models.iter() {
                    eprintln!("    \"{}\"    ({} existing notes)", &m.name, m.note_count);
                }
            } else {
                eprintln!("The database contains no models!");
            }
            process::exit(1);
        }
    };

    if opts.free.len() < 1 {
        eprintln!("Insufficient arguments provided");
        eprintln!("");
        print_usage(program, &opts_spec);
        process::exit(1);
    }

    let command = &opts.free[0];
    let command_args = &opts.free[1..];
    match String::from(command).to_lowercase().as_str() {
        "add" => {
            println!("Adding: {:?}", command_args);

            if command_args.len() != input_model.fields {
                eprintln!("Model '{}' expected {} fields but {} were provided", model_name, input_model.fields, command_args.len());
                process::exit(1);
            }

            write_new_entry_to_db(&sql, command_args, input_model.id, input_deck.id, &notes);
        }
        _ => {
            eprintln!("Command '{}' is unrecognised. Valid options are: 'add'", command);
            process::exit(1);
        }
    }

    if should_store_config {
        write_configuration(&config_path, &config);
    }
}
