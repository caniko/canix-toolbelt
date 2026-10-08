# The pinned Fleetix source owns the positional policy renderer and its
# operator-bound parity fixture. The Toolbelt adapter is exercised separately.
{inputs}: import "${inputs.fleetix}/tests/build-train-policy.nix"
