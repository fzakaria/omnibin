# `nix fmt`. The tree wrapper rather than bare `nixfmt`, extended so every
# language in the tree has a formatter rather than only the Nix code: rustfmt
# for src/, black for tools/, prettier for markdown. CI's one formatting step
# then covers the whole repository.
{ pkgs }:
pkgs.nixfmt-tree.override {
  runtimeInputs = [
    pkgs.rustfmt
    pkgs.black
    pkgs.prettier
  ];
  settings = {
    formatter.rustfmt = {
      command = "rustfmt";
      options = [
        "--edition"
        "2021"
      ];
      includes = [ "*.rs" ];
    };
    # black rather than a linter: the point is that nobody argues about where
    # a call wraps, and CI's one formatting step is what settles it.
    formatter.black = {
      command = "black";
      options = [ "--quiet" ];
      includes = [ "*.py" ];
    };
    # Markdown at the width the docs are already written to. proseWrap is left
    # at its default of preserving the author's line breaks: these are
    # hand-wrapped prose, and reflowing them would make every future diff a
    # whole-file diff.
    formatter.prettier-markdown = {
      command = "prettier";
      options = [
        "--write"
        "--print-width"
        "80"
      ];
      includes = [ "*.md" ];
    };
  };
}
