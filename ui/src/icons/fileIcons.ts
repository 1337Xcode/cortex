// Which icon stands for a file: the mark of its language or format, by the file's whole name for
// the names a tool owns (`Cargo.toml`, `package.json`, `Dockerfile`), else by its extension. Only
// icons drawn in one colour are used. A file whose kind has no such mark in thesvg has no icon, and
// nothing stands in for one: Java and C# (their marks are drawn only in colour), SQL (no mark), and
// XML and Less (their marks are lettering too small to read at the size the map draws icons).
// Each icon is imported by its own path, so the page carries only the icons listed here.
import Astro from '@thesvg/react/astro';
import Bun from '@thesvg/react/bun';
import CLanguage from '@thesvg/react/c';
import Clojure from '@thesvg/react/clojure';
import Cmake from '@thesvg/react/cmake';
import Cplusplus from '@thesvg/react/cplusplus';
import Crystal from '@thesvg/react/crystal';
import Css from '@thesvg/react/css';
import Dart from '@thesvg/react/dart';
import Deno from '@thesvg/react/deno';
import Docker from '@thesvg/react/docker';
import Dotenv from '@thesvg/react/dotenv';
import Elixir from '@thesvg/react/elixir';
import Elm from '@thesvg/react/elm';
import Erlang from '@thesvg/react/erlang';
import Fsharp from '@thesvg/react/fsharp';
import Git from '@thesvg/react/git';
import Gleam from '@thesvg/react/gleam';
import GnuBash from '@thesvg/react/gnu-bash';
import Go from '@thesvg/react/go';
import Gradle from '@thesvg/react/gradle';
import Graphql from '@thesvg/react/graphql';
import Haskell from '@thesvg/react/haskell';
import Html5 from '@thesvg/react/html5';
import Javascript from '@thesvg/react/javascript';
import Json from '@thesvg/react/json';
import Julia from '@thesvg/react/julia';
import Kotlin from '@thesvg/react/kotlin';
import Latex from '@thesvg/react/latex';
import Lua from '@thesvg/react/lua';
import Markdown from '@thesvg/react/markdown';
import Nim from '@thesvg/react/nim';
import Nixos from '@thesvg/react/nixos';
import Npm from '@thesvg/react/npm';
import Ocaml from '@thesvg/react/ocaml';
import Perl from '@thesvg/react/perl';
import Php from '@thesvg/react/php';
import Pnpm from '@thesvg/react/pnpm';
import Python from '@thesvg/react/python';
import RLanguage from '@thesvg/react/r';
import Ruby from '@thesvg/react/ruby';
import Rust from '@thesvg/react/rust';
import Sass from '@thesvg/react/sass';
import Scala from '@thesvg/react/scala';
import Solidity from '@thesvg/react/solidity';
import Svelte from '@thesvg/react/svelte';
import SvgFormat from '@thesvg/react/svg';
import Swift from '@thesvg/react/swift';
import Terraform from '@thesvg/react/terraform';
import Toml from '@thesvg/react/toml';
import Typescript from '@thesvg/react/typescript';
import Vim from '@thesvg/react/vim';
import Vuedotjs from '@thesvg/react/vuedotjs';
import Webassembly from '@thesvg/react/webassembly';
import Yaml from '@thesvg/react/yaml';
import Yarn from '@thesvg/react/yarn';
import Zig from '@thesvg/react/zig';
import Zsh from '@thesvg/react/zsh';
import { brandIcon, type BrandIcon } from './icon';

/** One icon and the keys (file names or extensions) it stands for. */
type Keyed = readonly [icon: BrandIcon, keys: readonly string[]];

/** A table from each key to its icon. */
function tableOf(rows: readonly Keyed[]): ReadonlyMap<string, BrandIcon> {
  return new Map(
    rows.flatMap(([icon, keys]) => keys.map((key): [string, BrandIcon] => [key, icon])),
  );
}

const RUST = brandIcon('rust', Rust, 'mono');
const TYPESCRIPT = brandIcon('typescript', Typescript, 'mono');
const PYTHON = brandIcon('python', Python, 'mono');
const GO = brandIcon('go', Go, 'mono');
const RUBY = brandIcon('ruby', Ruby, 'mono');
const DOCKER = brandIcon('docker', Docker, 'mono');
const GRADLE = brandIcon('gradle', Gradle, 'mono');
const NIX = brandIcon('nixos', Nixos, 'mono');

/** Whole file names that belong to a tool, matched exactly, before any extension. */
const BY_NAME = tableOf([
  [RUST, ['Cargo.toml', 'Cargo.lock']],
  [brandIcon('npm', Npm, 'mono'), ['package.json', 'package-lock.json', '.npmrc']],
  [brandIcon('pnpm', Pnpm, 'mono'), ['pnpm-lock.yaml', 'pnpm-workspace.yaml']],
  [brandIcon('yarn', Yarn, 'mono'), ['yarn.lock', '.yarnrc.yml']],
  [TYPESCRIPT, ['tsconfig.json']],
  [brandIcon('deno', Deno, 'mono'), ['deno.json', 'deno.jsonc', 'deno.lock']],
  [brandIcon('bun', Bun, 'mono'), ['bun.lock', 'bun.lockb', 'bunfig.toml']],
  [GO, ['go.mod', 'go.sum', 'go.work']],
  [PYTHON, ['pyproject.toml', 'requirements.txt', 'Pipfile', 'Pipfile.lock']],
  [RUBY, ['Gemfile', 'Gemfile.lock', 'Rakefile']],
  [
    DOCKER,
    [
      'Dockerfile',
      '.dockerignore',
      'compose.yaml',
      'compose.yml',
      'docker-compose.yaml',
      'docker-compose.yml',
    ],
  ],
  [brandIcon('cmake', Cmake, 'mono'), ['CMakeLists.txt']],
  [GRADLE, ['build.gradle', 'settings.gradle', 'build.gradle.kts', 'settings.gradle.kts']],
  [brandIcon('git', Git, 'mono'), ['.gitignore', '.gitattributes', '.gitmodules']],
  [brandIcon('dotenv', Dotenv, 'mono'), ['.env']],
  [NIX, ['flake.lock']],
]);

/** Extensions, lower case and without their dot. */
const BY_EXTENSION = tableOf([
  [RUST, ['rs']],
  [TYPESCRIPT, ['ts', 'mts', 'cts', 'tsx']],
  [brandIcon('javascript', Javascript, 'mono'), ['js', 'mjs', 'cjs', 'jsx']],
  [PYTHON, ['py', 'pyi']],
  [GO, ['go']],
  [brandIcon('c', CLanguage, 'mono'), ['c', 'h']],
  [brandIcon('cplusplus', Cplusplus, 'mono'), ['cpp', 'cc', 'cxx', 'hpp', 'hh', 'hxx']],
  [RUBY, ['rb']],
  [brandIcon('php', Php, 'mono'), ['php']],
  [brandIcon('kotlin', Kotlin, 'mono'), ['kt', 'kts']],
  [brandIcon('swift', Swift, 'mono'), ['swift']],
  [brandIcon('scala', Scala, 'mono'), ['scala']],
  [brandIcon('gnu-bash', GnuBash, 'mono'), ['sh', 'bash']],
  [brandIcon('zsh', Zsh, 'mono'), ['zsh']],
  [brandIcon('json', Json, 'mono'), ['json', 'jsonc']],
  [brandIcon('yaml', Yaml, 'mono'), ['yaml', 'yml']],
  [brandIcon('toml', Toml, 'mono'), ['toml']],
  [brandIcon('markdown', Markdown, 'mono'), ['md', 'markdown']],
  [brandIcon('html5', Html5, 'mono'), ['html', 'htm']],
  [brandIcon('css', Css, 'mono'), ['css']],
  [brandIcon('sass', Sass, 'mono'), ['scss', 'sass']],
  [brandIcon('vuedotjs', Vuedotjs, 'mono'), ['vue']],
  [brandIcon('svelte', Svelte, 'mono'), ['svelte']],
  [brandIcon('astro', Astro, 'mono'), ['astro']],
  [brandIcon('lua', Lua, 'mono'), ['lua']],
  [brandIcon('dart', Dart, 'mono'), ['dart']],
  [brandIcon('elixir', Elixir, 'mono'), ['ex', 'exs']],
  [brandIcon('erlang', Erlang, 'mono'), ['erl', 'hrl']],
  [brandIcon('haskell', Haskell, 'mono'), ['hs']],
  [brandIcon('zig', Zig, 'mono'), ['zig']],
  [brandIcon('nim', Nim, 'mono'), ['nim']],
  [brandIcon('r', RLanguage, 'mono'), ['r']],
  [brandIcon('perl', Perl, 'mono'), ['pl', 'pm']],
  [brandIcon('ocaml', Ocaml, 'mono'), ['ml', 'mli']],
  [brandIcon('clojure', Clojure, 'mono'), ['clj', 'cljs', 'cljc']],
  [brandIcon('julia', Julia, 'mono'), ['jl']],
  [brandIcon('solidity', Solidity, 'mono'), ['sol']],
  [brandIcon('graphql', Graphql, 'mono'), ['graphql', 'gql']],
  [brandIcon('terraform', Terraform, 'mono'), ['tf']],
  [brandIcon('fsharp', Fsharp, 'mono'), ['fs', 'fsi', 'fsx']],
  [brandIcon('webassembly', Webassembly, 'mono'), ['wasm', 'wat']],
  [brandIcon('elm', Elm, 'mono'), ['elm']],
  [brandIcon('gleam', Gleam, 'mono'), ['gleam']],
  [brandIcon('crystal', Crystal, 'mono'), ['cr']],
  [brandIcon('latex', Latex, 'mono'), ['tex']],
  [NIX, ['nix']],
  [brandIcon('svg', SvgFormat, 'mono'), ['svg']],
  [brandIcon('vim', Vim, 'mono'), ['vim']],
  [GRADLE, ['gradle']],
  [DOCKER, ['dockerfile']],
]);

/**
 * The icon of the file at `path` (a repository path with `/` between folders): by its whole name,
 * else by its extension in any case, else null. A name that only starts with a dot (`.bashrc`) has
 * no extension.
 */
export function fileIcon(path: string): BrandIcon | null {
  const name = path.slice(path.lastIndexOf('/') + 1);
  const byName = BY_NAME.get(name);
  if (byName !== undefined) return byName;
  const dot = name.lastIndexOf('.');
  if (dot <= 0) return null;
  return BY_EXTENSION.get(name.slice(dot + 1).toLowerCase()) ?? null;
}

/**
 * The icon of a row that names two files: the one both have, or null when they differ, so a row
 * of a Rust file and a TypeScript file shows neither mark rather than one of them.
 */
export function sharedFileIcon(a: string, b: string): BrandIcon | null {
  const icon = fileIcon(a);
  return icon !== null && icon.name === fileIcon(b)?.name ? icon : null;
}
