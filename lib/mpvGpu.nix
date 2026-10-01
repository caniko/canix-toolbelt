# mpv's media adapter deliberately leaves presentation-device selection alone.
media: let
  enabled = media.enable or false;
  nvidia = enabled && (media.vendor or null) == "nvidia";
in
  {
    hwdec =
      if nvidia
      then "nvdec-copy"
      else if enabled
      then "vaapi-copy"
      else "auto-safe";
  }
  // (
    if enabled && !nvidia
    then {vaapi-device = media.renderNode;}
    else {}
  )
