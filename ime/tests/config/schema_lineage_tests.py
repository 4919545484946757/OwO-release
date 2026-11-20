"""Regression coverage for the independently released native schema lineages."""
import pathlib
import subprocess
import sys
import tempfile

MASTER_FIELDS = {
    16: ("smart_candidates_enabled", "expressive_candidates_enabled", "concept_candidates_enabled"),
    17: ("corpus_learning_enabled", "corpus_learning_paused", "corpus_learning_allowed_apps",
         "corpus_learning_excluded_apps", "corpus_retention_days"),
    18: ("corpus_prediction_enabled",),
    19: ("style_scene_mask", "style_preferences"),
    20: ("tone_reply_enabled", "tone_continuation_enabled", "tone_cloud_model_enabled"),
}
RELEASE_FIELDS = (
    "candidate_single_character_first_enabled",
    "expanded_previous_page_shortcut_enabled", "expanded_previous_page_shortcut",
    "expanded_next_page_shortcut_enabled", "expanded_next_page_shortcut",
)

def main():
    shell = pathlib.Path(sys.argv[1]).resolve()
    checks = 0
    def run(path, *arguments, success=True):
        nonlocal checks
        if not success:
            pathlib.Path(str(path) + ".bak").unlink(missing_ok=True)
        result = subprocess.run([str(shell), str(path), *arguments],
                                capture_output=True, encoding="utf-8", timeout=15)
        actual = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
        assert result.returncode == 0, (arguments, result.returncode, result.stderr)
        if not success:
            assert "configuration and backup invalid; using defaults" in result.stderr, result.stderr
            assert actual == defaults, "Invalid configuration must fall back to safe defaults"
        checks += 1
        return actual
    def encode(values):
        return "".join(f"{key}={value}\n" for key, value in values.items())
    with tempfile.TemporaryDirectory(prefix="owo-schema-lineages-") as folder:
        path = pathlib.Path(folder) / "owo.conf"
        defaults = run(path, "defaults")
        assert defaults["schema_version"] == "21"
        for version in range(16, 21):
            master = dict(defaults, schema_version=str(version), candidate_page_size="3",
                          smart_candidates_enabled="false",
                          previous_page_shortcut="[;Up", next_page_shortcut="];Down")
            for key in RELEASE_FIELDS:
                master.pop(key)
            for introduced, keys in MASTER_FIELDS.items():
                if introduced > version:
                    for key in keys:
                        master.pop(key)
            if version >= 17:
                master.update(corpus_learning_enabled="true", corpus_learning_paused="true",
                              corpus_learning_allowed_apps="WeChat.exe;Code.exe", corpus_retention_days="37")
            if version >= 19:
                master.update(style_scene_mask="2", style_preferences="short~~precise")
            if version >= 20:
                master.update(tone_reply_enabled="true", tone_cloud_model_enabled="false")
            original = encode(master)
            path.write_text(original, encoding="utf-8")
            loaded = run(path, "show")
            assert path.read_text(encoding="utf-8") == original, "Showing old settings must not rewrite them"
            assert loaded["smart_candidates_enabled"] == "false"
            assert loaded["previous_page_shortcut"] == "[" and loaded["expanded_previous_page_shortcut"] == "Up"
            for key, expected in master.items():
                if key not in ("schema_version", "previous_page_shortcut", "next_page_shortcut"):
                    assert loaded[key] == expected, (version, key, loaded[key], expected)
            run(path, "set", "candidate_page_size", "4")
            saved = run(path, "show")
            assert saved["schema_version"] == "21" and saved["candidate_page_size"] == "4"
            assert saved["smart_candidates_enabled"] == "false"
            if version >= 17:
                assert saved["corpus_retention_days"] == "37" and saved["corpus_learning_paused"] == "true"
            checks += 1

        for version in (16, 17):
            release = dict(defaults, schema_version=str(version),
                           candidate_single_character_first_enabled="false", candidate_page_size="5")
            for keys in MASTER_FIELDS.values():
                for key in keys:
                    release.pop(key)
            if version == 16:
                for key in RELEASE_FIELDS[1:]:
                    release.pop(key)
                release.update(previous_page_shortcut="[;Up", next_page_shortcut="];Down")
            else:
                release.update(expanded_previous_page_shortcut="Ctrl+F7",
                               expanded_next_page_shortcut="Ctrl+F8",
                               expanded_next_page_shortcut_enabled="false")
            path.write_text(encode(release), encoding="utf-8")
            loaded = run(path, "show")
            assert loaded["candidate_single_character_first_enabled"] == "false"
            assert loaded["corpus_learning_enabled"] == "false"
            assert loaded["tone_reply_enabled"] == "false" and loaded["tone_cloud_model_enabled"] == "false"
            if version == 17:
                assert loaded["expanded_previous_page_shortcut"] == "Ctrl+F7"
                assert loaded["expanded_next_page_shortcut_enabled"] == "false"
            run(path, "set", "candidate_page_size", "6")
            assert run(path, "show")["candidate_single_character_first_enabled"] == "false"
            for missing in ("candidate_single_character_first_enabled", "expanded_next_page_shortcut"):
                if missing in release:
                    malformed = dict(release)
                    malformed.pop(missing)
                    path.write_text(encode(malformed), encoding="utf-8")
                    run(path, "show", success=False)
            mixed = dict(release, smart_candidates_enabled="true")
            path.write_text(encode(mixed), encoding="utf-8")
            run(path, "show", success=False)
            checks += 1
        for field in ("tone_cloud_model_enabled", "expanded_previous_page_shortcut",
                      "candidate_single_character_first_enabled", "corpus_learning_paused"):
            malformed = dict(defaults)
            malformed.pop(field)
            path.write_text(encode(malformed), encoding="utf-8")
            run(path, "show", success=False)
        path.write_text(encode(defaults) + "schema_version=21\n", encoding="utf-8")
        run(path, "show", success=False)
    print(f"Schema lineage migration and rejection checks passed: {checks}")

if __name__ == "__main__":
    main()
