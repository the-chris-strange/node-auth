use node_auth::{Options, run};
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[tokio::test]
async fn test_end_to_end_run_with_explicit_token() {
    let dir = tempdir().unwrap();
    let repo_npmrc = dir.path().join("project.npmrc");
    let cred_npmrc = dir.path().join("user.npmrc");

    fs::write(
        &repo_npmrc,
        "@test-scope:registry=https://us-central1-npm.pkg.dev/my-proj/my-repo/\nstrict-ssl=true\n",
    )
    .unwrap();

    let options = Options {
        repo_config: Some(repo_npmrc.clone()),
        credential_config: Some(cred_npmrc.clone()),
        token: Some("integration-test-token-adc".to_string()),
        ..Default::default()
    };

    let result = run(&options).await;
    assert!(result.is_ok(), "run(&options) failed: {:?}", result.err());

    let cred_content = fs::read_to_string(&cred_npmrc).unwrap();
    assert!(cred_content.contains(
        "//us-central1-npm.pkg.dev/my-proj/my-repo/:_authToken=\"integration-test-token-adc\""
    ));

    let repo_content = fs::read_to_string(&repo_npmrc).unwrap();
    assert!(
        repo_content
            .contains("@test-scope:registry=https://us-central1-npm.pkg.dev/my-proj/my-repo/")
    );
    assert!(!repo_content.contains("_authToken"));
}

#[tokio::test]
async fn test_end_to_end_yarn_integration() {
    let dir = tempdir().unwrap();
    let repo_yarn = dir.path().join(".yarnrc.yml");
    let cred_yarn = dir.path().join("user.yarnrc.yml");
    let repo_npmrc = dir.path().join(".npmrc");
    let cred_npmrc = dir.path().join("user.npmrc");

    fs::write(
        &repo_npmrc,
        "@test-scope:registry=https://us-central1-npm.pkg.dev/my-proj/my-repo/\n",
    )
    .unwrap();

    fs::write(
        &repo_yarn,
        r#"
npmScopes:
  test-scope:
    npmRegistryServer: "https://us-central1-npm.pkg.dev/my-proj/my-repo"
"#,
    )
    .unwrap();

    let options = Options {
        repo_config: Some(repo_npmrc),
        credential_config: Some(cred_npmrc),
        repo_config_yarn: Some(repo_yarn),
        credential_config_yarn: Some(cred_yarn.clone()),
        yarn: Some(true),
        token: Some("yarn-token-xyz".to_string()),
        ..Default::default()
    };

    let result = run(&options).await;
    assert!(result.is_ok(), "run(&options) failed: {:?}", result.err());

    let yarn_content = fs::read_to_string(&cred_yarn).unwrap();
    assert!(yarn_content.contains("test-scope"));
    assert!(yarn_content.contains("npmAuthToken: yarn-token-xyz"));
}

#[tokio::test]
async fn equivalent_npm_paths_are_written_once() {
    let dir = tempdir().unwrap();
    let path = dir.path().join(".npmrc");
    fs::write(
        &path,
        "@scope:registry=https://us-npm.pkg.dev/project/repo/\n",
    )
    .unwrap();
    let options = Options {
        repo_config: Some(dir.path().join("./.npmrc")),
        credential_config: Some(path.clone()),
        token: Some("valid-token".to_string()),
        ..Default::default()
    };
    run(&options).await.unwrap();
    assert!(
        fs::read_to_string(path)
            .unwrap()
            .contains(":_authToken=\"valid-token\"")
    );
}

#[tokio::test]
async fn invalid_yarn_preserves_both_npm_files() {
    let dir = tempdir().unwrap();
    let npm_source = dir.path().join("project.npmrc");
    let npm_target = dir.path().join("user.npmrc");
    let yarn_source = dir.path().join("project.yarnrc.yml");
    let yarn_target = dir.path().join("user.yarnrc.yml");
    let source_before = "@scope:registry=https://us-npm.pkg.dev/project/repo/\n//us-npm.pkg.dev/project/repo/:_authToken=old\n";
    let target_before = "save-exact=true\n";
    fs::write(&npm_source, source_before).unwrap();
    fs::write(&npm_target, target_before).unwrap();
    fs::write(
        &yarn_source,
        "npmScopes:\n  scope:\n    npmRegistryServer: https://us-npm.pkg.dev/project/repo\n",
    )
    .unwrap();
    fs::write(&yarn_target, "invalid: [\n").unwrap();
    let options = Options {
        repo_config: Some(npm_source.clone()),
        credential_config: Some(npm_target.clone()),
        repo_config_yarn: Some(yarn_source),
        credential_config_yarn: Some(yarn_target.clone()),
        yarn: Some(true),
        token: Some("valid-token".to_string()),
        ..Default::default()
    };
    let error = run(&options).await.unwrap_err();
    assert!(error.to_string().contains("credential .yarnrc.yml"));
    assert_eq!(fs::read_to_string(npm_source).unwrap(), source_before);
    assert_eq!(fs::read_to_string(npm_target).unwrap(), target_before);
    assert_eq!(fs::read_to_string(yarn_target).unwrap(), "invalid: [\n");
}

#[tokio::test]
async fn yarn_does_not_attach_token_to_other_registry_without_override() {
    let dir = tempdir().unwrap();
    let npm_source = dir.path().join("project.npmrc");
    let yarn_source = dir.path().join("project.yarnrc.yml");
    let yarn_target = dir.path().join("user.yarnrc.yml");
    fs::write(
        &npm_source,
        "@scope:registry=https://us-npm.pkg.dev/project/repo/\n",
    )
    .unwrap();
    fs::write(
        &yarn_source,
        "npmScopes:\n  other:\n    npmRegistryServer: https://registry.example.com/private\n",
    )
    .unwrap();
    let options = Options {
        repo_config: Some(npm_source),
        credential_config: Some(dir.path().join("user.npmrc")),
        repo_config_yarn: Some(yarn_source),
        credential_config_yarn: Some(yarn_target.clone()),
        yarn: Some(true),
        token: Some("valid-token".to_string()),
        ..Default::default()
    };
    run(&options).await.unwrap();
    assert!(!yarn_target.exists());

    let override_options = Options {
        allow_all_domains: true,
        ..options
    };
    run(&override_options).await.unwrap();
    assert!(
        fs::read_to_string(yarn_target)
            .unwrap()
            .contains("npmAuthToken: valid-token")
    );
}

#[test]
fn both_cli_names_share_presentation_and_keep_errors_on_stderr() {
    for binary in [
        env!("CARGO_BIN_EXE_node-auth"),
        env!("CARGO_BIN_EXE_artifactregistry-auth"),
    ] {
        let printed = Command::new(binary)
            .args(["--print-token", "--token", "valid-token"])
            .output()
            .unwrap();
        assert!(printed.status.success());
        assert_eq!(String::from_utf8(printed.stdout).unwrap(), "valid-token\n");
        assert!(printed.stderr.is_empty());

        let dir = tempdir().unwrap();
        let failed = Command::new(binary)
            .arg("--repo-config")
            .arg(dir.path().join("missing.npmrc"))
            .arg("--credential-config")
            .arg(dir.path().join("user.npmrc"))
            .args(["--token", "valid-token"])
            .output()
            .unwrap();
        assert!(!failed.status.success());
        assert!(failed.stdout.is_empty());
        assert_eq!(
            String::from_utf8(failed.stderr)
                .unwrap()
                .matches("Error:")
                .count(),
            1
        );
    }
}

#[test]
fn compatibility_binary_accepts_official_yarn_flags_without_npm_config() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("project.yarnrc.yml");
    let target = dir.path().join("user.yarnrc.yml");
    fs::write(
        &source,
        "npmScopes:\n  scope:\n    npmRegistryServer: https://us-npm.pkg.dev/project/repo\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_artifactregistry-auth"))
        .arg("--repo-config")
        .arg(dir.path().join("missing.npmrc"))
        .arg("--credential-config")
        .arg(dir.path().join("user.npmrc"))
        .arg("--repo-config-yarn")
        .arg(&source)
        .arg("--credential-config-yarn")
        .arg(&target)
        .args(["--token", "valid-token"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!dir.path().join("user.npmrc").exists());
    assert!(
        fs::read_to_string(target)
            .unwrap()
            .contains("npmAuthToken: valid-token")
    );
}
