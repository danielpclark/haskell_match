# Changelog

All notable changes to haskell_match are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Haskell tier 1.** User-defined operators with `infixl`/`infixr`/`infix`
  declarations (prefix, infix and backtick definitions; sections, operator
  values, `where`-bound operators; callable from Ruby); infix constructors
  (`data V = Double :| Double`, `(:+:) a b`, infix rendering) in Haskell
  source and in `HaskellMatch.data`; record field selectors, record
  construction and update syntax; pattern guards and `let` guards;
  `MultiWayIf`, `LambdaCase`, `TupleSections` and `NamedFieldPuns`; the
  full set of character escapes and string gaps; left sections (previously
  mis-parsed).
- **Modules.** `import` declarations between compiled modules (items,
  `hiding`, qualified/`as` accepted), with imported types joining the
  importing module's type scope (`Native.import_scope`,
  `HaskellMatch::Haskell.import_into`), export lists (`haskell_exports`),
  `haskell_values`, `haskell_types`, `haskell_imports`, module names
  resolving to constants or `Dir/File.hs` on `$LOAD_PATH`, and standard
  library module names resolving to the Prelude.

- **Inline Haskell.** `HaskellMatch.haskell(source)` compiles a Haskell 2010
  subset (data declarations, equations, guards, `where`, `let`, `case`,
  lambdas, sections, list comprehensions, ranges) into methods on a Ruby
  module; `extend HaskellMatch::Haskell` gives a module body a `haskell`
  method. Exhaustiveness and redundancy are enforced for every function,
  `where` helper, `case` and pattern lambda, with errors located in the Ruby
  or `.hs` file.
- **Haskell files.** `HaskellMatch.load(path)` compiles a plain `.hs` file
  into a module; `HaskellMatch.require(name)` finds `name.hs` on
  `$LOAD_PATH` and defines a constant named after the `module` header.
- **A lazy Prelude** (`HaskellMatch::Prelude`) with the standard list,
  numeric, character and `Maybe`/`Either`/`Ordering` functions, usable from
  Ruby as well as from compiled Haskell.
- **Lazy cons.** `LazyList.lazy_cons(head) { tail }` and `LazyList.deferred`
  build cells whose tail is computed on demand; `LazyList#==` compares
  elements. Compiled Haskell uses them for `x : e`, so `primes = sieve [2..]`
  and `fibs = 0 : 1 : zipWith (+) fibs (tail fibs)` work as written.
- **Type scopes.** `HaskellMatch.new_scope` and the `scope:` option of
  `data`, `fn`, `case_of`, `pattern` and `hdef`. Each compiled Haskell module
  gets its own scope (a snapshot of the global registry plus its own types)
  and exposes `fn`, `case_of`, `pattern` and `data` in it, so two modules can
  both declare a `Shape`.
- `Function#curried`: a memoised curried Proc.
- `HaskellMatch.constructor(name)`: the current registration of a constructor.
- A type module forwards `new`, `[]` and `call` to a same-named constructor
  (`Person.new(...)` for `data Person = Person {...}`).
- `on(..., location: [file, line])` to report a clause's origin.
- **Hash patterns**: `{name = n, "key" = p}` (quoted) or a Ruby Hash literal
  in place, matching a Hash that has the listed keys, with full
  exhaustiveness and redundancy checking.
- **Ruby classes as constructors**: `HaskellMatch.sealed(:Shape, Circle,
  Rect, Tri => %i[a b c])` makes existing `Data`, `Struct` or plain classes
  the constructors of a closed type; a `Data`/`Struct` class visible from a
  definition is registered automatically as a single-constructor type when
  a pattern names it.
- **`deriving (Ord, Enum, Bounded)`** on data declarations (`Eq`/`Show`
  already hold): comparison by constructor order, `succ`/`pred`/ranges,
  `Type.enum_from*`, `Type.min_bound`/`max_bound`. Unsupported classes are
  rejected before anything is registered.
- **Checked field types**: `HaskellMatch.data "...", check_types: true` (or
  `HaskellMatch.check_field_types = true`) verifies constructor arguments
  against the declared Haskell types and raises `FieldTypeError`.
- **`where` helpers** inside `fn` definitions: local checked functions
  reachable by name from the clause bodies (`Function#helpers`).
- `Function#>>` / `#<<` composition; `Function#scope`.
- Pattern syntax errors show the pattern with a caret; Haskell syntax errors
  show the offending source line with a caret.
- `Native.parse_data` returns field types and the `deriving` list;
  `Constructor.field_types`.

### Changed

- `Native::Matcher.new` takes a fifth argument, the type scope;
  `Native.register_type`, `render_pattern`, `constructors` and
  `constructor_info` take an optional scope.

## [0.1.0] - unreleased

### Added

- Haskell pattern syntax, quoted or written in place in Ruby blocks, compiled
  to Maranget decision trees in Rust (Rutie).
- Algebraic data types with `HaskellMatch.data`.
- Exhaustiveness and redundancy checking with GHC-style messages.
- Strings as lists of characters; lazy lists and Enumerators as lists.
- Recursion beyond the VM stack: fiber stack segments, `tail`, `defer`,
  `max_depth`, and `deep: true` mode.
- Thread, fiber and Ractor safety, with `ractor: true` functions.
