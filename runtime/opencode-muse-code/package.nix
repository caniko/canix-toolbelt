package:
package.overrideAttrs (old: {
  # visibility.patch also backports the filesystem cycle fix from caniko/opencode a515e138d8.
  patches = (old.patches or []) ++ [./visibility.patch ./transform.patch];
})
