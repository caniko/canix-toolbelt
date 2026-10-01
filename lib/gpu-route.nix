# The same precedence for integrated NixOS/HM and standalone HM consumers.
{
  role,
  args,
  config,
}: let
  osConfig = config._module.args.osConfig or null;
  option =
    if role == "media"
    then "gpuMedia"
    else "gpuRender";
  inherited =
    if osConfig == null
    then null
    else osConfig.canix-toolbelt.${option} or null;
  topologyDefault = (args.gpuRoutes or {}).${role} or (args.${option} or {});
in
  if inherited != null
  then inherited
  else topologyDefault
