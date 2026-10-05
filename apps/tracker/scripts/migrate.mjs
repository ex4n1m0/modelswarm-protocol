// Forward-only migration runner. Applies every migrations/*.sql in order to
// the database named by DATABASE_URL, recording applied files in
// schema_migrations. Without DATABASE_URL it is a no-op (CI/dev without
// Postgres still exit 0).
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const migrationsDir = join(here, "..", "migrations");

const connectionString = process.env.DATABASE_URL;

if (!connectionString) {
  console.log("skipped (no DATABASE_URL)");
  process.exit(0);
}

const pg = (await import("pg")).default;
const client = new pg.Client({ connectionString });

async function ensureMigrationTable() {
  await client.query(`
    CREATE TABLE IF NOT EXISTS schema_migrations (
      name       varchar(256) PRIMARY KEY,
      applied_at timestamptz NOT NULL DEFAULT now()
    )
  `);
}

async function appliedSet() {
  const result = await client.query("SELECT name FROM schema_migrations");
  return new Set(result.rows.map((row) => row.name));
}

async function main() {
  const files = readdirSync(migrationsDir)
    .filter((f) => f.endsWith(".sql"))
    .sort();
  if (files.length === 0) {
    console.log("no migration files found");
    return;
  }
  await client.connect();
  try {
    await ensureMigrationTable();
    const done = await appliedSet();
    for (const file of files) {
      if (done.has(file)) {
        console.log(`skip ${file} (already applied)`);
        continue;
      }
      const sql = readFileSync(join(migrationsDir, file), "utf8");
      await client.query("BEGIN");
      try {
        await client.query(sql);
        await client.query("INSERT INTO schema_migrations (name) VALUES ($1)", [file]);
        await client.query("COMMIT");
        console.log(`apply ${file}`);
      } catch (error) {
        await client.query("ROLLBACK");
        throw new Error(`migration ${file} failed: ${error instanceof Error ? error.message : String(error)}`);
      }
    }
  } finally {
    await client.end();
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
