pub mod export;
pub mod hash_password;
pub mod healthcheck;
pub mod import;
pub mod init;
pub mod rerender;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "subprime-garden", version, about = "A single-user blogging engine")]
pub struct Cli {
    /// Path to the TOML config file. Falls back to $SUBPRIME_CONFIG, then
    /// ./garden.toml if that exists, then env vars alone.
    #[arg(long, global = true, env = "SUBPRIME_CONFIG")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// --config / $SUBPRIME_CONFIG, else ./garden.toml if present.
    pub fn config_path(&self) -> Option<PathBuf> {
        self.config.clone().or_else(|| {
            let default = PathBuf::from("garden.toml");
            default.is_file().then_some(default)
        })
    }
}

#[derive(Subcommand)]
pub enum Command {
    /// Run the HTTP server.
    Serve,
    /// Scaffold templates/static and walk garden.toml to a complete config.
    Init {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Overwrite existing files and re-run the full prompt sequence.
        #[arg(long)]
        force: bool,
        /// Prompt for a new admin password even if one is already set.
        #[arg(long)]
        reset_password: bool,
    },
    /// Scaffold or refresh templates/static only — never touches garden.toml.
    Theme {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Overwrite existing template/static files with the stock theme.
        #[arg(long)]
        force: bool,
    },
    /// Run pending database migrations and exit.
    Migrate,
    /// Import a directory of markdown files (Zola/Hugo/Jekyll frontmatter accepted).
    Import {
        dir: PathBuf,
        #[arg(long)]
        published: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Export the database back out as markdown files with YAML frontmatter.
    Export { dir: PathBuf },
    /// Rebuild stored HTML for every post (after markdown option changes).
    Rerender,
    /// Internal: check /healthz over a raw TCP connection. Used by the container healthcheck.
    Healthcheck,
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::db::models::PostStatus;
    use crate::db::{media, posts, taxonomy};

    fn conn() -> rusqlite::Connection {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::run_migrations(&mut conn).unwrap();
        conn
    }

    fn config() -> Config {
        Config { collections: crate::config::test_collections(), ..Config::default() }
    }

    fn new_post(slug: &str, title: &str, status: PostStatus, collection: &str, fields: serde_json::Value) -> posts::NewPost {
        posts::NewPost {
            slug: slug.into(),
            title: title.into(),
            markdown: format!("Body of {title}."),
            html: String::new(),
            excerpt: String::new(),
            status,
            collection: collection.into(),
            fields: fields.as_object().unwrap().clone(),
        }
    }

    fn add_cover(conn: &rusqlite::Connection) {
        media::insert(
            conn,
            &media::NewMedia {
                filename: "cover.png".into(),
                original_name: "cover.png".into(),
                mime: "image/png".into(),
                bytes: 1,
                width: Some(10),
                height: Some(20),
                variant_filename: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn export_then_import_brings_back_collection_fields_status_and_taxonomies() {
        let dir = std::env::temp_dir().join(format!("subprime-garden-test-{}", crate::auth::random_token()));
        let book_fields = serde_json::json!({
            "author": "Ursula K. Le Guin",
            "rating": 5,
            "finished": "2026-09-14",
            "cover": "cover.png",
            "shelf": "read",
        });

        let source = conn();
        add_cover(&source);
        posts::insert(&source, &new_post("about", "About", PostStatus::Published, "pages", serde_json::json!({}))).unwrap();
        posts::insert(&source, &new_post("dispossessed", "The Dispossessed", PostStatus::Published, "books", book_fields.clone())).unwrap();
        let id = posts::insert(&source, &new_post("hello", "Hello", PostStatus::Draft, "posts", serde_json::json!({}))).unwrap();
        let terms = taxonomy::find_or_create(&source, "tags", &["rust".to_string()]).unwrap();
        taxonomy::set_post_terms(&source, "tags", id, &terms).unwrap();
        super::export::run(&source, &dir).unwrap();

        // Media fields need the media store restored first.
        let mut target = conn();
        let opts = super::import::ImportOptions { published: false, force: false, dry_run: false };
        super::import::run(&mut target, &dir, &opts, &config()).unwrap();
        assert!(posts::get_by_slug(&target, "dispossessed", true).unwrap().is_none());
        add_cover(&target);
        super::import::run(&mut target, &dir, &opts, &config()).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        let about = posts::get_by_slug(&target, "about", true).unwrap().unwrap();
        assert_eq!((about.collection.as_str(), about.status), ("pages", PostStatus::Published));
        assert_eq!((about.title.as_str(), about.markdown.trim_end()), ("About", "Body of About."));

        let book = posts::get_by_slug(&target, "dispossessed", true).unwrap().unwrap();
        assert_eq!(book.collection, "books");
        assert_eq!(serde_json::Value::Object(book.fields), book_fields);

        let hello = posts::get_by_slug(&target, "hello", true).unwrap().unwrap();
        assert_eq!((hello.collection.as_str(), hello.status), ("posts", PostStatus::Draft));
        assert_eq!(taxonomy::names_csv_for_post(&target, "tags", hello.id).unwrap(), "rust");
    }

    #[test]
    fn import_rejects_unknown_collections_and_fields() {
        let dir = std::env::temp_dir().join(format!("subprime-garden-test-{}", crate::auth::random_token()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "---\ncollection: recipes\n---\nBody.\n").unwrap();
        std::fs::write(dir.join("b.md"), "---\ncollection: books\nfields:\n  pages: 300\n---\nBody.\n").unwrap();
        std::fs::write(dir.join("c.md"), "---\ncollection: books\nfields:\n  rating: five\n---\nBody.\n").unwrap();

        let mut target = conn();
        let opts = super::import::ImportOptions { published: false, force: false, dry_run: false };
        super::import::run(&mut target, &dir, &opts, &config()).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert!(posts::list_all(&target, None).unwrap().is_empty());
    }
}
