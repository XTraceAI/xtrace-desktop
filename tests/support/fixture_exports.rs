pub fn assert_fixture_export_matches_golden(actual: &str, golden: &str, package_version: &str) {
    let actual_metadata: serde_json::Value = serde_json::from_str(actual).unwrap();
    assert_eq!(actual_metadata["app_info"]["version"], package_version);
    assert_eq!(actual, golden_with_package_version(golden, package_version));
}

pub fn golden_with_package_version(golden: &str, package_version: &str) -> String {
    // Fail closed if the committed AppInfo header changes shape. Never round-trip
    // the body: parsed floats are not round-trip exact under serde_json defaults.
    const PREFIX: &str = "{\n  \"app_info\": {\n";
    let (header, _) = golden
        .strip_prefix(PREFIX)
        .expect("golden must start with the AppInfo header")
        .split_once("\n  },\n")
        .expect("golden must delimit the AppInfo header exactly");
    assert_eq!(header.matches("\"version\"").count(), 1);
    let metadata: serde_json::Value = serde_json::from_str(&format!("{{\n{header}\n}}"))
        .expect("golden AppInfo header must be valid JSON");
    let version = metadata["version"]
        .as_str()
        .expect("golden AppInfo version must be a string");
    let token = format!("    \"version\": \"{version}\",\n");
    assert_eq!(header.matches(&token).count(), 1);
    let start = PREFIX.len() + header.find(&token).unwrap() + "    \"version\": \"".len();
    let mut expected = golden.to_owned();
    expected.replace_range(start..start + version.len(), package_version);
    expected
}
