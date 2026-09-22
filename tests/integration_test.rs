use std::fs;
use tempfile::tempdir;
use node_auth::{run, Options};

#[tokio::test]
async fn test_end_to_end_run_with_verbose_and_explicit_token() {
    let dir = tempdir().unwrap();
    let repo_npmrc = dir.path().join("project.npmrc");
    let cred_npmrc = dir.path().join("user.npmrc");

    fs::write(
        &repo_npmrc,
        "@test-scope:registry=https://us-central1-npm.pkg.dev/my-proj/my-repo/\nstrict-ssl=true\n",
    ).unwrap();

    let options = Options {
        repo_config: Some(repo_npmrc.clone()),
        credential_config: Some(cred_npmrc.clone()),
        token: Some("integration-test-token-adc".to_string()),
        verbose: true,
        ..Default::default()
    };

    let result = run(&options).await;
    assert!(result.is_ok(), "run(&options) failed: {:?}", result.err());

    let cred_content = fs::read_to_string(&cred_npmrc).unwrap();
    assert!(cred_content.contains("//us-central1-npm.pkg.dev/my-proj/my-repo/:_authToken=\"integration-test-token-adc\""));

    let repo_content = fs::read_to_string(&repo_npmrc).unwrap();
    assert!(repo_content.contains("@test-scope:registry=https://us-central1-npm.pkg.dev/my-proj/my-repo/"));
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
    ).unwrap();

    fs::write(
        &repo_yarn,
        r#"
npmScopes:
  test-scope:
    npmRegistryServer: "https://us-central1-npm.pkg.dev/my-proj/my-repo"
"#,
    ).unwrap();

    let options = Options {
        repo_config: Some(repo_npmrc),
        credential_config: Some(cred_npmrc),
        repo_config_yarn: Some(repo_yarn),
        credential_config_yarn: Some(cred_yarn.clone()),
        yarn: Some(true),
        token: Some("yarn-token-xyz".to_string()),
        verbose: true,
        ..Default::default()
    };

    let result = run(&options).await;
    assert!(result.is_ok(), "run(&options) failed: {:?}", result.err());

    let yarn_content = fs::read_to_string(&cred_yarn).unwrap();
    assert!(yarn_content.contains("test-scope"));
    assert!(yarn_content.contains("npmAuthToken: yarn-token-xyz"));
}