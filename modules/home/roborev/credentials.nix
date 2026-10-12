{
  lib,
  settings,
  runtimeSecretPath,
  envReference,
}: let
  get = path: lib.attrByPath path "" settings;
  embedding = get ["search" "embeddings" "api_key"];
  embeddingSource =
    if builtins.isAttrs embedding
    then embedding
    else {};
  envName = embeddingSource.env or "";
  keyFile = embeddingSource.file or "";
  appKey = get ["ci" "github_app_private_key"];
  postgres = get ["sync" "postgres_url"];
  withoutReferences = value: lib.concatStrings (builtins.filter builtins.isString (builtins.split "[$][{][^}]+[}]" value));
  postgresLogin = builtins.match "postgres(ql)?://([^/@:]+)(:([^/@]*))?@.*" (withoutReferences postgres);
  postgresPassword =
    if postgresLogin == null
    then null
    else builtins.elemAt postgresLogin 3;
  postgresReferences = builtins.filter builtins.isList (builtins.split "[$][{]([^}]+)[}]" postgres);
  validReference = reference: let
    value = builtins.head reference;
  in
    if lib.hasPrefix "file:" value
    then runtimeSecretPath (lib.removePrefix "file:" value)
    else builtins.match "[A-Za-z_][A-Za-z0-9_]*" value != null;
  hooks = settings.hooks or [];
in [
  {
    assertion = (settings.auth_key or "") == "";
    message = "Use roborev.runtimeConfigFile for auth_key; never serialize a daemon key in settings.";
  }
  {
    assertion = get ["web" "auth_token"] == "";
    message = "Use web.auth_token_file or runtimeConfigFile for browser credentials.";
  }
  {
    assertion = get ["web" "auth_token_file"] == "" || runtimeSecretPath (get ["web" "auth_token_file"]);
    message = "web.auth_token_file must be an absolute runtime path outside the store.";
  }
  {
    assertion = (settings.anthropic_api_key or "") == "" || envReference settings.anthropic_api_key;
    message = "anthropic_api_key must be an upstream environment reference; literal keys require runtimeConfigFile.";
  }
  {
    assertion = appKey == "" || envReference appKey || runtimeSecretPath appKey;
    message = "ci.github_app_private_key must be a runtime file path or environment reference.";
  }
  {
    assertion = embedding == "" || (builtins.isAttrs embedding && (embeddingSource.value or "") == "" && ((envName != "" && keyFile == "" && builtins.match "[A-Za-z_][A-Za-z0-9_]*" envName != null) || (envName == "" && keyFile != "" && runtimeSecretPath keyFile)));
    message = "search.embeddings.api_key requires exactly one env name or runtime file path; literal credentials require runtimeConfigFile.";
  }
  {
    assertion = (postgresPassword == null || postgresPassword == "") && builtins.match ".*[?&]password=[^&]+.*" (withoutReferences postgres) == null && builtins.all validReference postgresReferences;
    message = "sync.postgres_url passwords require upstream environment/file references outside the store or runtimeConfigFile.";
  }
  {
    assertion = get ["ci" "discord_webhook_url"] == "";
    message = "Store ci.discord_webhook_url in private runtimeConfigFile.";
  }
  {
    assertion = builtins.all (hook: (hook.url or "") == "") hooks;
    message = "Store sensitive hooks[].url values in private runtimeConfigFile.";
  }
]
