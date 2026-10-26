# Reusable Roborev Home Manager integration
# Aggregates the program and service modules.
{
  imports = [
    ./program.nix
    ./service.nix
    ./repositories.nix
    ./integrations.nix
  ];
}
