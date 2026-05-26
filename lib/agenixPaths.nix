{
  mkAgenixPaths = {root}: {
    host = hostname: filename: "${root}/hosts/${hostname}/${filename}.age";
    user = username: filename: "${root}/users/${username}/${filename}.age";
    module = path: "${root}/modules/${path}.age";
    project = projectName: name: "${root}/${projectName}/${name}.age";
  };
}
