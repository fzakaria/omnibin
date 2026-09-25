# The deployable site: the static files from site/, the rendered docs, and the
# data files built from the pinned databases.
#
# The data is built here rather than fetched at page load, so the page and the
# index it describes always deploy together. The databases come from the
# release data-pins.json names, which is what keeps the pair together.
{ pkgs, self }:
let
  inherit (pkgs) lib;

  # The commit stamped into the footer. From a clean checkout self.rev names
  # exactly the tree the data came from; a dirty tree gets dirtyRev; anything
  # else keeps the placeholder and the footer line stays hidden.
  commit = self.rev or self.dirtyRev or "__COMMIT__";

  docs = import ./docs.nix { inherit pkgs; };

  pins = builtins.fromJSON (builtins.readFile ../data-pins.json);

  # Every system with a published database, taken from the pins rather than
  # named here, so publishing a new system needs no edit to this file.
  systems = lib.filter (name: lib.hasPrefix "omnibin-" name && lib.hasSuffix ".db" name) (
    builtins.attrNames pins.files
  );

  databases = map (
    name:
    (import ./data.nix {
      inherit pkgs;
      system = lib.removeSuffix ".db" (lib.removePrefix "omnibin-" name);
    }).database
  ) systems;

  data =
    pkgs.runCommand "omnibin-site-data"
      {
        nativeBuildInputs = [ pkgs.python3 ];
      }
      ''
        mkdir -p $out
        python3 ${../tools/build-site-data.py} \
          --db ${lib.concatStringsSep " " databases} \
          --multiverse-tag "${pins.multiverseTag or "unknown"}" \
          --out $out
      '';
in
pkgs.runCommand "omnibin-site" { } ''
  mkdir -p $out/docs
  cp -r ${../site}/* $out/
  cp -r ${data}/* $out/
  cp -r ${docs}/* $out/docs/
  chmod -R u+w $out

  substituteInPlace $out/js/app.js --replace-quiet "__COMMIT__" "${commit}"
  substituteInPlace $out/js/app.js --replace-fail "__STORE_PATH__" "$out"
  for f in $out/docs/*.html; do
    substituteInPlace "$f" --replace-quiet "__COMMIT__" "${commit}"
    substituteInPlace "$f" --replace-fail "__STORE_PATH__" "$out"
  done

  # Content-hash the module directory after the substitutions, so the HTML and
  # the script it loads can never be a mismatched pair across deploys.
  hash=$(find $out/js -type f -name '*.js' | LC_ALL=C sort |
    xargs sha256sum | sha256sum | cut -c1-12)
  mv $out/js "$out/js.$hash"
  substituteInPlace $out/index.html --replace-fail "./js/app.js" "./js.$hash/app.js"
''
