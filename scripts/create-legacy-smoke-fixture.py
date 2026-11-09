"""Create an isolated v1 upgrade fixture; never touch the live application data."""
import argparse
import json
from pathlib import Path
import shutil
import sqlite3


def read_only(database: Path):
    return sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)


def create_fixture(output: Path, report_path: Path | None):
    root = Path(__file__).resolve().parent.parent
    artifacts = (root / "artifacts").resolve()
    output = output.resolve()
    if not output.is_relative_to(artifacts) or output.exists():
        raise ValueError("Use a new database path inside artifacts; existing files are never overwritten")
    output.parent.mkdir(parents=True, exist_ok=True)
    if report_path:
        report = json.loads(report_path.read_text(encoding="utf-8"))
        source = (Path(report["dataDir"]) / "fluxdl.sqlite3").resolve()
        if not source.is_relative_to(artifacts):
            raise ValueError("Only isolated smoke databases inside artifacts may be copied")
        with read_only(source) as connection:
            if connection.execute("PRAGMA user_version").fetchone()[0] != 1:
                raise ValueError("The source must be a genuine schema-v1 database")
        shutil.copyfile(source, output)
    else:
        media_dir = str(output.parent / "legacy-downloads")
        settings = dict(downloadDir=media_dir, enginePath="", ffmpegPath="", defaultMode="video", quality="720", theme="light", density="compact", reducedMotion=False, closeBehavior="tray", completionAction="none")
        task = dict(id="legacy-completed-fixture", media=dict(url="https://example.invalid/legacy.mp4", title="旧数据库兼容性夹具", mediaId="legacy", source="Synthetic compatibility fixture", duration=None, thumbnail=None, isPlaylist=False, formats=[], warnings=[]), mode="video", quality="720", formatId=None, outputDir=media_dir, status="completed", progress=None, speed=None, eta=None, downloadedBytes=1024, totalBytes=1024, finalPath=str(Path(media_dir) / "missing-legacy.mp4"), error=None, logs=[], createdAt="2026-01-01T00:00:00Z", updatedAt="2026-01-01T00:00:00Z", fileExists=False)
        with sqlite3.connect(output) as connection:
            connection.executescript("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; CREATE TABLE app_settings(id INTEGER PRIMARY KEY CHECK(id=1),document TEXT NOT NULL); CREATE TABLE tasks(id TEXT PRIMARY KEY,document TEXT NOT NULL); PRAGMA user_version=1;")
            connection.execute("INSERT INTO app_settings VALUES(1,?)", (json.dumps(settings, ensure_ascii=False),))
            connection.execute("INSERT INTO tasks VALUES(?,?)", (task["id"], json.dumps(task, ensure_ascii=False)))
    with sqlite3.connect(output) as connection:
        settings = json.loads(connection.execute("SELECT document FROM app_settings WHERE id=1").fetchone()[0])
        settings["futurePreference"] = {"retained": "engineering-upgrade-fixture"}
        connection.execute("UPDATE app_settings SET document=? WHERE id=1", (json.dumps(settings, ensure_ascii=False),))
        template = json.loads(connection.execute("SELECT document FROM tasks ORDER BY id LIMIT 1").fetchone()[0])
        template.update(id="legacy-active-upgrade-fixture", status="queued", futureTopLevel={"retained": "engineering-upgrade-fixture"})
        template["media"]["futureMediaField"] = 42
        connection.execute("INSERT INTO tasks VALUES(?,?)", (template["id"], json.dumps(template, ensure_ascii=False)))
        template.update(id="legacy-unknown-state-upgrade-fixture", status="future-reserved-state")
        connection.execute("INSERT INTO tasks VALUES(?,?)", (template["id"], json.dumps(template, ensure_ascii=False)))
        rows = connection.execute("SELECT COUNT(*) FROM tasks").fetchone()[0]
    return {"database": output.relative_to(root).as_posix(), "schema": 1, "rows": rows, "source": "copied isolated 0.1 desktop database" if report_path else "documented v1 schema with synthetic compatibility records"}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("artifacts/legacy-upgrade/fluxdl.sqlite3"))
    parser.add_argument("--from-report", type=Path)
    args = parser.parse_args()
    print(json.dumps(create_fixture(args.output, args.from_report), ensure_ascii=False))
