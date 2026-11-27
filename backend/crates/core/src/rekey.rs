//! `tokuly rekey`: what was sealed under `TOKULY_SECRET_PREVIOUS` is sealed
//! again under `TOKULY_SECRET`, so that the previous secret can go.
//!
//! Every domain lists the columns it seals ([`Sealed`], a `SEALED` constant
//! of its crate); the server binary hands them all to [`rekey`]. A row is
//! rewritten only if it still holds the value that was read, so the command
//! may run while the servers do: what they seal meanwhile is under the new
//! secret already.
//!
//! Keyed hashes of secrets that are not stored (API tokens, client
//! secrets, recovery codes) cannot be computed again: they are accepted
//! under the previous secret while it is set, and rewritten when used.
use sqlx::PgPool;

use crate::keys::{Keys, b64url, b64url_decode};

/// A column holding values made with [`Keys::seal`].
#[derive(Debug, Clone, Copy)]
pub struct Sealed {
    pub table: &'static str,
    pub column: &'static str,
    pub label: Label,
    /// `bytea`, or base64url in a `text` column.
    pub text: bool,
    /// A column with [`Keys::token_hash`] of the plaintext, written again too.
    pub hash: Option<&'static str>,
}

/// The label a column's values are sealed with.
#[derive(Debug, Clone, Copy)]
pub enum Label {
    Fixed(&'static str),
    /// An SQL expression over the row (one label per row).
    Sql(&'static str),
}

impl Sealed {
    /// A `bytea` column sealed with `label`.
    pub const fn bytes(table: &'static str, column: &'static str, label: &'static str) -> Self {
        Self { table, column, label: Label::Fixed(label), text: false, hash: None }
    }

    /// A `text` column of base64url sealed with `label`.
    pub const fn text(table: &'static str, column: &'static str, label: &'static str) -> Self {
        Self { text: true, ..Self::bytes(table, column, label) }
    }

    /// With a column of the plaintext's keyed hash, rewritten along.
    pub const fn hashed(self, hash: &'static str) -> Self {
        Self { hash: Some(hash), ..self }
    }

    fn label_sql(&self) -> String {
        match self.label {
            Label::Fixed(label) => format!("'{}'", label.replace('\'', "''")),
            Label::Sql(sql) => sql.to_owned(),
        }
    }
}

/// What the core seals: console secrets (`settings::store`).
pub const SEALED: &[Sealed] = &[Sealed {
    table: "server_settings",
    column: "sealed",
    label: Label::Sql("'settings/' || section || '.' || key"),
    text: false,
    hash: None,
}];

/// What [`rekey`] did to one column.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Rekeyed {
    /// Sealed again under the current secret.
    pub resealed: u64,
    /// Opens with neither secret (it was sealed under some other one): left as it is.
    pub unopened: u64,
}

const BATCH: i64 = 500;

/// Seals again under the current secret every value of `column` that only
/// the previous secret opens.
pub async fn rekey(db: &PgPool, keys: &Keys, column: &Sealed) -> anyhow::Result<Rekeyed> {
    anyhow::ensure!(keys.has_previous(), "TOKULY_SECRET_PREVIOUS is not set: there is nothing to seal again");
    let Sealed { table, column: name, text, hash, .. } = *column;
    let stored = if text { format!("{name}::text") } else { format!("encode({name}, 'base64')") };
    let select = format!(
        "SELECT ctid::text, {label}, {stored} FROM {table} WHERE {name} IS NOT NULL AND ctid > $1::tid ORDER BY ctid LIMIT {BATCH}",
        label = column.label_sql(),
    );
    // Values travel as text: base64url as stored, or base64 of the bytes.
    let (new, old) = if text { ("$1", "$2") } else { ("decode($1, 'base64')", "decode($2, 'base64')") };
    let and_hash = hash.map(|hash| format!(", {hash} = $4")).unwrap_or_default();
    let update = format!("UPDATE {table} SET {name} = {new}{and_hash} WHERE ctid = $3::tid AND {name} = {old}");
    let mut done = Rekeyed::default();
    let mut after = "(0,0)".to_owned();
    loop {
        let rows: Vec<(String, String, String)> =
            sqlx::query_as(sqlx::AssertSqlSafe(select.clone())).bind(&after).fetch_all(db).await?;
        let Some(last) = rows.last() else { break };
        after = last.0.clone();
        for (ctid, label, encoded) in rows {
            let sealed = if text { b64url_decode(&encoded) } else { base64_decode(&encoded) };
            let Some(sealed) = sealed else {
                done.unopened += 1;
                continue;
            };
            let Some(plain) = keys.open_previous(&label, &sealed) else {
                done.unopened += u64::from(keys.open(&label, &sealed).is_none());
                continue;
            };
            let resealed = keys.seal(&label, &plain);
            let resealed = if text { b64url(&resealed) } else { base64_encode(&resealed) };
            let mut query = sqlx::query(sqlx::AssertSqlSafe(update.clone())).bind(resealed).bind(encoded).bind(ctid);
            if hash.is_some() {
                query = query.bind(keys.token_hash(std::str::from_utf8(&plain)?));
            }
            done.resealed += query.execute(db).await?.rows_affected();
        }
    }
    Ok(done)
}

fn base64_decode(encoded: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    // PostgreSQL's `encode(…, 'base64')` breaks lines every 76 characters.
    let joined: String = encoded.split_whitespace().collect();
    base64::engine::general_purpose::STANDARD.decode(joined).ok()
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
