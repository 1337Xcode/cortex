// The files' icons: a path for every name and extension the table knows, with the thesvg icon it
// is drawn with, written out by hand; the paths that get none; and every icon drawn in the text's
// colour alone and taken from the module its name says.
import { describe, expect, it } from 'vitest';
import { fileIcon, sharedFileIcon } from './fileIcons';
import { expectDrawnInTextColour, expectIsThesvgIcon } from './testIcons';

/** One path for each whole name the table matches, and the icon it is drawn with. */
const BY_NAME: readonly (readonly [string, string])[] = [
  ['Cargo.toml', 'rust'],
  ['crates/core/Cargo.toml', 'rust'],
  ['Cargo.lock', 'rust'],
  ['package.json', 'npm'],
  ['ui/package-lock.json', 'npm'],
  ['.npmrc', 'npm'],
  ['pnpm-lock.yaml', 'pnpm'],
  ['pnpm-workspace.yaml', 'pnpm'],
  ['yarn.lock', 'yarn'],
  ['.yarnrc.yml', 'yarn'],
  ['tsconfig.json', 'typescript'],
  ['deno.json', 'deno'],
  ['deno.jsonc', 'deno'],
  ['deno.lock', 'deno'],
  ['bun.lock', 'bun'],
  ['bun.lockb', 'bun'],
  ['bunfig.toml', 'bun'],
  ['go.mod', 'go'],
  ['go.sum', 'go'],
  ['go.work', 'go'],
  ['pyproject.toml', 'python'],
  ['requirements.txt', 'python'],
  ['Pipfile', 'python'],
  ['Pipfile.lock', 'python'],
  ['Gemfile', 'ruby'],
  ['Gemfile.lock', 'ruby'],
  ['Rakefile', 'ruby'],
  ['Dockerfile', 'docker'],
  ['.dockerignore', 'docker'],
  ['compose.yaml', 'docker'],
  ['compose.yml', 'docker'],
  ['docker-compose.yaml', 'docker'],
  ['docker-compose.yml', 'docker'],
  ['CMakeLists.txt', 'cmake'],
  ['build.gradle', 'gradle'],
  ['settings.gradle', 'gradle'],
  ['build.gradle.kts', 'gradle'],
  ['settings.gradle.kts', 'gradle'],
  ['.gitignore', 'git'],
  ['.gitattributes', 'git'],
  ['.gitmodules', 'git'],
  ['.env', 'dotenv'],
  ['flake.lock', 'nixos'],
];

/** One file for each extension the table matches, and the icon it is drawn with. */
const BY_EXTENSION: readonly (readonly [string, string])[] = [
  ['src/lib.rs', 'rust'],
  ['a.ts', 'typescript'],
  ['a.mts', 'typescript'],
  ['a.cts', 'typescript'],
  ['App.tsx', 'typescript'],
  ['types.d.ts', 'typescript'],
  ['a.js', 'javascript'],
  ['a.mjs', 'javascript'],
  ['a.cjs', 'javascript'],
  ['a.jsx', 'javascript'],
  ['a.py', 'python'],
  ['a.pyi', 'python'],
  ['main.go', 'go'],
  ['a.c', 'c'],
  ['a.h', 'c'],
  ['a.cpp', 'cplusplus'],
  ['a.cc', 'cplusplus'],
  ['a.cxx', 'cplusplus'],
  ['a.hpp', 'cplusplus'],
  ['a.hh', 'cplusplus'],
  ['a.hxx', 'cplusplus'],
  ['a.rb', 'ruby'],
  ['a.php', 'php'],
  ['a.kt', 'kotlin'],
  ['a.kts', 'kotlin'],
  ['a.swift', 'swift'],
  ['a.scala', 'scala'],
  ['a.sh', 'gnu-bash'],
  ['a.bash', 'gnu-bash'],
  ['a.zsh', 'zsh'],
  ['a.json', 'json'],
  ['a.jsonc', 'json'],
  ['a.yaml', 'yaml'],
  ['a.yml', 'yaml'],
  ['a.toml', 'toml'],
  ['README.md', 'markdown'],
  ['a.markdown', 'markdown'],
  ['a.html', 'html5'],
  ['a.htm', 'html5'],
  ['a.css', 'css'],
  ['a.scss', 'sass'],
  ['a.sass', 'sass'],
  ['a.vue', 'vuedotjs'],
  ['a.svelte', 'svelte'],
  ['a.astro', 'astro'],
  ['a.lua', 'lua'],
  ['a.dart', 'dart'],
  ['a.ex', 'elixir'],
  ['a.exs', 'elixir'],
  ['a.erl', 'erlang'],
  ['a.hrl', 'erlang'],
  ['a.hs', 'haskell'],
  ['a.zig', 'zig'],
  ['a.nim', 'nim'],
  ['a.r', 'r'],
  ['a.pl', 'perl'],
  ['a.pm', 'perl'],
  ['a.ml', 'ocaml'],
  ['a.mli', 'ocaml'],
  ['a.clj', 'clojure'],
  ['a.cljs', 'clojure'],
  ['a.cljc', 'clojure'],
  ['a.jl', 'julia'],
  ['a.sol', 'solidity'],
  ['a.graphql', 'graphql'],
  ['a.gql', 'graphql'],
  ['main.tf', 'terraform'],
  ['a.fs', 'fsharp'],
  ['a.fsi', 'fsharp'],
  ['a.fsx', 'fsharp'],
  ['a.wasm', 'webassembly'],
  ['a.wat', 'webassembly'],
  ['a.elm', 'elm'],
  ['a.gleam', 'gleam'],
  ['a.cr', 'crystal'],
  ['a.tex', 'latex'],
  ['default.nix', 'nixos'],
  ['a.svg', 'svg'],
  ['a.vim', 'vim'],
  ['init.gradle', 'gradle'],
  ['app.dockerfile', 'docker'],
];

describe('the files', () => {
  it.each([...BY_NAME, ...BY_EXTENSION])('draws %s with %s', (path, name) => {
    expect(fileIcon(path)?.name).toBe(name);
  });

  it('matches an extension in any case, and a whole name only as written', () => {
    expect(fileIcon('src/MAIN.RS')?.name).toBe('rust');
    expect(fileIcon('R/analysis.R')?.name).toBe('r');
    // Not Cargo's own file name, so it is drawn as the TOML it is.
    expect(fileIcon('cargo.toml')?.name).toBe('toml');
  });

  it.each([
    ['src/Main.java', 'Java: its mark is drawn only in colour'],
    ['Program.cs', 'C#: its marks are drawn only in colour'],
    ['schema.sql', 'SQL: no mark'],
    ['pom.xml', 'XML: lettering too small to read'],
    ['site.less', 'Less: lettering too small to read'],
    ['Makefile', 'no mark for make'],
    ['README', 'no extension'],
    ['.bashrc', 'a name that only starts with a dot has no extension'],
    ['Dockerfile.dev', 'not the whole name, and .dev is no extension the table knows'],
    ['src/a.rs/notes', 'a dot in a folder is not the file extension'],
    ['archive.', 'an empty extension'],
    ['', 'no name'],
  ])('draws %s with no icon (%s)', (path) => {
    expect(fileIcon(path)).toBeNull();
  });

  it('gives a row of two files the icon both share, or none', () => {
    expect(sharedFileIcon('src/orders/book.rs', 'src/orders/mod.rs')?.name).toBe('rust');
    expect(sharedFileIcon('Cargo.toml', 'src/lib.rs')?.name).toBe('rust');
    expect(sharedFileIcon('src/lib.rs', 'ui/app.ts')).toBeNull();
    expect(sharedFileIcon('Makefile', 'README')).toBeNull();
  });

  it.each([...new Set([...BY_NAME, ...BY_EXTENSION].map(([, name]) => name))])(
    "draws %s in the text's colour alone, from its own module",
    async (name) => {
      const path = [...BY_NAME, ...BY_EXTENSION].find(([, icon]) => icon === name)?.[0] ?? '';
      const icon = fileIcon(path);
      expect(icon?.name).toBe(name);
      if (icon === null) return;
      expectDrawnInTextColour(icon);
      await expectIsThesvgIcon(icon);
    },
  );
});
