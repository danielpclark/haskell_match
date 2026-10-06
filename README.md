# haskell_match

Haskell's pattern matching for Ruby, implemented in Rust with
[Rutie](https://github.com/danielpclark/rutie).

* **Haskell pattern syntax**, as strings: constructors, literals, variables,
  wildcards, lists `[]` / `(x:xs)` / `[a, b]`, tuples, as-patterns `all@(x:_)`,
  lazy patterns `~p`, bang patterns `!x`, record patterns
  `Person { name = n, .. }`, guards and `otherwise`.
* **Algebraic data types** declared with Haskell `data` syntax; constructors
  are frozen `Data` values with Haskell-style `inspect`.
* **Every logical path accounted for.** Like GHC, the compiler rejects a
  function whose clauses are not exhaustive and lists the patterns not
  matched; it also rejects clauses that can never be reached. Positions whose
  patterns disagree in type are a compile error. At run time a value that no
  pattern of the function could accept raises a type-mismatch error (the
  error Haskell's type checker would have given), while `_` and variables
  match anything, exactly as in Haskell.
* **Patterns with or without quotes.** Write Haskell syntax in a string, or
  write the pattern in place as Ruby: `on([x, *xs])`, `on(Just(Just(_)))`,
  `on(Person(name: n))`. Both forms build the same pattern and can be mixed.
* **Lazy lists.** `(x:xs)` patterns match `HaskellMatch.lazy(1..)` and Ruby
  Enumerators, so infinite lists work as they do in Haskell.
* **Recursion without Ruby's stack limit.** Plain recursion is carried across
  chained Fiber stacks, tail calls run in constant space, and a depth guard
  turns a runaway recursion into an error instead of exhausted memory.
* **Fast.** Clauses are compiled once (Maranget-style) into a decision tree
  that inspects each argument position at most once per path, and matching
  runs in Rust directly over Ruby `VALUE`s with no allocation until a clause
  is chosen. List tails are shared slices, not copies.

```ruby
require "haskell_match"

HaskellMatch.data "Maybe a = Nothing | Just a"
HaskellMatch.data "Shape = Circle Double | Rect Double Double"
include Maybe
include Shape

from_maybe = HaskellMatch.fn(:from_maybe) do
  on("d", "Nothing") { |d| d }
  on("_", "Just x")  { |x| x }
end

from_maybe.(0, Just.new(5))   # => 5
from_maybe.(0, Nothing)       # => 0

area = HaskellMatch.fn(:area) do
  on("Circle r") { |r| 3.14159 * r * r }
  on("Rect w h") { |w, h| w * h }
end

length = HaskellMatch.fn(:length) do
  on("[]")     { 0 }
  on("(_:xs)") { |xs| 1 + length.(xs) }
end
length.([1, 2, 3])            # => 3

# the same, with the patterns written in place
length = HaskellMatch.fn(:length) do
  on([])        { 0 }
  on([_, *xs])  { |xs| 1 + length.(xs) }
end
```

Leave a case out and the definition fails, exactly where Haskell would warn:

```ruby
HaskellMatch.fn(:area) do
  on("Circle r") { |r| 3.14159 * r * r }
end
# HaskellMatch::NonExhaustiveError:
# Pattern match(es) are non-exhaustive
# In an equation for 'area':
#     Patterns not matched:
#         Rect _ _

HaskellMatch.fn(:silly) do
  on("_")      { 1 }
  on("Just x") { |x| x }
end
# HaskellMatch::RedundantClauseError:
# Pattern match is redundant
# In an equation for 'silly':
#     silly Just x = ... (example.rb:3)
```

## Installation

You need Ruby ≥ 3.2 and a Rust toolchain (`cargo`). The gem builds the
extension on install:

```sh
gem install haskell_match      # or add it to your Gemfile
```

From a checkout:

```sh
bundle install
bundle exec rake compile       # cargo build --release, copies the library into lib/
bundle exec rake test          # Ruby test suite (compiles first)
bundle exec rake cargo:test    # Rust unit tests for the pure core
bundle exec rake bench
```

The library is loaded with `Fiddle`; set `HASKELL_MATCH_NATIVE` to point at a
specific build if needed.

## Declaring data types

```ruby
HaskellMatch.data "Maybe a = Nothing | Just a"
HaskellMatch.data "Either a b = Left a | Right b"
HaskellMatch.data "Tree a = Leaf | Node (Tree a) a (Tree a)"
HaskellMatch.data "Person = Person { name :: String, age :: Int }"
HaskellMatch.data "Color = Red | Green | Blue deriving (Show, Eq)"   # deriving is accepted and ignored

# the same, without the declaration syntax
HaskellMatch.data :Maybe, Nothing: 0, Just: 1
HaskellMatch.data :Person, Person: { name: :String, age: :Int }
```

`HaskellMatch.data` defines a module named after the type (as a top-level
constant by default; pass `under: SomeModule`, or `under: nil` to get it back
without defining a constant). The module holds one constant per constructor,
so `include Maybe` brings `Just` and `Nothing` into scope.

* Constructors with fields are `Data` classes: `Just.new(1)`, `Just[1]`,
  `Just.(1)`, `[1, 2].map(&Just)`. Positional fields are `_1`, `_2`, ...;
  record fields have their own names. Values are frozen, compare by value,
  hash, and print Haskell-style: `Just (Just 1)`, `Person {name = "Ann", age = 30}`.
  They also work with Ruby's own `case/in` (`deconstruct_keys`).
* Nullary constructors are singleton values: `Nothing`, `Red`.
* `Maybe === value` tests membership; `Maybe.constructors` lists them.
* Types are closed: declaring `data Pet = Dog | Cat` later with a different
  set of constructors replaces the type; existing compiled functions keep
  working against the old values.

Haskell keeps types and constructors in different namespaces; Ruby does not.
For `data Person = Person {...}`, after `include Person` the name `Person`
refers to the constructor; the type module is `::Person` or
`Person.data_type`.

## Patterns

| Haskell                          | Matches                                                    |
|----------------------------------|------------------------------------------------------------|
| `x`, `_`, `_name`                | anything (variables bind, `_` does not)                    |
| `Just x`, `Nothing`, `Node l v r`| a constructor and its fields                               |
| `Person { name = n, age }`       | record fields by name (`age` is a pun); `Person { .. }` binds all |
| `[]`, `[a, b]`, `(x:xs)`, `x:y:rest` | Ruby Arrays viewed as lists                             |
| `(a, b)`, `(a, b, c)`, `()`      | Ruby Arrays of exactly that length                         |
| `True`, `False`                  | `true`, `false`                                            |
| `0`, `-1`, `1.5`, `0xFF`, `12345678901234567890` | numbers (`0` also matches `0.0`, as in Haskell) |
| `'c'`                            | a character: a one-character Ruby String, or one character of a String matched as a list |
| `"text"`                         | the list `['t', 'e', 'x', 't']`: `String = [Char]`, so `f "" = ...; f (c:cs) = ...` works on Ruby Strings |
| `:sym`, `:"quoted"`              | Ruby Symbols (an extension)                                |
| `all@(x:_)`                      | as-pattern                                                 |
| `~(a, b)`                        | lazy (irrefutable) pattern: always matches; destructured only when the clause runs |
| `!x`                             | bang pattern (accepted; Ruby is strict anyway)             |
| `Data.Maybe.Just x`              | qualification is ignored                                   |

Comments (`-- ...`, `{- ... -}`) are allowed inside patterns.

Ruby Strings are lists of characters wherever a list pattern appears: `[]`
matches `""`, `(c:cs)` binds `c` to a one-character String and `cs` to the
rest (a String), and `['y', _]` matches any two-character String starting
with `y`. The same patterns match Arrays of one-character Strings. `Char` and
`String` are different types, as in Haskell: `'a'` and `"a"` cannot appear in
the same position.

### Wildcards and run-time types

`_` and variables match any value and never fail. Constructor and literal
patterns fail on values of another type, which simply moves matching on to
the next clause. Only when no pattern of the function can accept a value is
`HaskellMatch::TypeMismatchError` raised, the run-time counterpart of the
compile error Haskell would give:

```ruby
f = HaskellMatch.fn(:f) { on("Just x") { |x| x }; on("_") { :other } }
f.(5)           # => :other           `_` accepts anything
g = HaskellMatch.fn(:g) { on("Just x") { |x| x }; on("Nothing") { 0 } }
g.(5)           # TypeMismatchError: expected a value of type Maybe but got 5 (Integer)
```

### Patterns without quotes

Inside a definition block the pattern can be written as a Ruby expression.
Bare lower-case names are variables, `_` is the wildcard, and the expression
is rendered to the Haskell syntax above and compiled identically, so
exhaustiveness checks, errors and speed are the same. Quoted and in-place
patterns can be mixed, even within one clause.

| In place                       | Haskell                     |
|--------------------------------|-----------------------------|
| `x`, `_`, `var(:name)`         | `x`, `_`, `name` (`var` for a name taken by a method or local) |
| `Just(x)`, `Just[x]`, `Nothing`| `Just x`, `Nothing`         |
| `Node(Leaf, v, Node(_, _, _))` | `Node Leaf v (Node _ _ _)`  |
| `Person(name: n, age: _)`      | `Person { name = n, age = _ }` |
| `Person(name: n, **_)`         | `Person { name = n, .. }`   |
| `[]`, `[a, b]`                 | `[]`, `[a, b]`              |
| `[x, *xs]`, `cons(x, xs)`      | `(x:xs)`                    |
| `[x, y, *_]`                   | `(x:y:_)`                   |
| `tuple(a, b)`, `unit`          | `(a, b)`, `()`              |
| `as(all, [x, *_])`             | `all@(x:_)`                 |
| `lazy(tuple(a, b))`, `bang(x)` | `~(a, b)`, `!x`             |
| `0`, `-1`, `1.5`, `true`, `:ok`| `0`, `-1`, `1.5`, `True`, `:ok` |
| `str("abc")`, `char("c")`      | `"abc"`, `'c'`              |

A String handed directly to `on` is quoted pattern syntax (`on("Just x")`);
inside a pattern a Ruby String is a string literal (`Just("abc")`), and
`str("abc")` makes one at the top level. A bare name that is also a method
of the enclosing object (or a local variable) is not a pattern variable;
use `var(:name)` for it. `HaskellMatch.pattern { Just([x, *_]) }` builds a
standalone pattern the same way.

### Guards

```ruby
sign = HaskellMatch.fn(:sign) do
  on("n", guard: ->(n) { n > 0 }) { 1 }
  on("n", guard: ->(n) { n < 0 }) { -1 }
  on("_", guard: otherwise)       { 0 }     # or simply on("_") { 0 }
end
```

A guarded clause does not count towards exhaustiveness (a guard may fail),
exactly as in GHC; `otherwise` does.

### Bindings

Bound variables are handed to the body in the order they appear in the
pattern. Name your block parameters after the variables and you may take any
subset in any order; keyword parameters work too:

```ruby
on("(x:xs)") { |xs| ... }            # by name
on("(x:xs)") { |xs, x| ... }         # reordered
on("(x:xs)") { |x:, xs:| ... }       # keywords
on("(x:xs)") { |**all| all }         # => { x: 1, xs: [2, 3] }
on("(x:xs)") { |*vals| vals }        # positional, pattern order
```

Naming a parameter that the pattern does not bind is a `DefinitionError`.

### Recursion

A function is in scope inside its own clauses under its name, and as `recur`:

```ruby
fact = HaskellMatch.fn(:fact) do
  on(0) { 1 }
  on(n) { |n| n * fact.(n - 1) }         # or recur.(n - 1)
end
```

Ruby's VM stack is fixed in size, so plain recursion written in Ruby dies at
roughly 10,000 levels. haskell_match removes that limit the way GHC's growable
stack does, with three mechanisms:

* **Stack segments (automatic).** Every `HaskellMatch.stack_segment` nested
  calls (default 100) the next clause body runs in a fresh Fiber, which brings
  its own VM and machine stacks. `1 + length.(xs)` therefore recurses as deep
  as memory allows with no change to the code. The cost is about 1.6 KB per
  level (each segment commits a 128 KB fiber VM stack), reclaimed when the
  call returns. Non-local exits (`throw`, `break`) do not cross segment
  boundaries.
* **Tail calls (constant space).** Return `function.tail(args...)` from a
  body and the call continues with those arguments on the same frame, like a
  Haskell loop:

  ```ruby
  sum = HaskellMatch.fn(:sum) do
    on(acc, [])        { |acc| acc }
    on(acc, [x, *xs])  { |acc, x, xs| sum.tail(acc + x, xs) }
  end
  sum.(0, (1..1_000_000).to_a)   # => 500000500000
  ```
* **Deferred calls (cheap depth).** `function.defer(args...) { |result| ... }`
  stands for a non-tail call whose result the block receives; the pending
  blocks are kept on a heap-backed stack managed natively, at about 200 bytes
  per level:

  ```ruby
  length = HaskellMatch.fn(:length) do
    on([])        { 0 }
    on([_, *xs])  { |xs| length.defer(xs) { |n| 1 + n } }   # 1 + length xs
  end
  ```

`HaskellMatch.max_depth` (default 250,000 nested calls, about 400 MB at the
segment cost) raises `HaskellMatch::StackOverflowError` beyond that depth, so
a runaway recursion fails instead of taking the machine's memory; set it
higher, or to 0 for no limit, when a computation legitimately needs more.
Tail calls do not count towards the depth.

### Lazy lists

`HaskellMatch.lazy(enumerable)` builds a memoised lazy list; list patterns
match it element by element, forcing only what they inspect, so infinite
lists work as in Haskell. Ruby Enumerators (including `Enumerator::Lazy`)
given to a function are wrapped automatically, iterating from the start each
time.

```ruby
take = HaskellMatch.fn(:take) do
  on(0, _)         { [] }
  on(_, [])        { [] }
  on(n, [x, *xs])  { |n, x, xs| [x] + take.(n - 1, xs) }
end
take.(5, HaskellMatch.lazy(1..))                       # => [1, 2, 3, 4, 5]
take.(4, HaskellMatch::LazyList.iterate(1) { |x| x * 2 })   # => [1, 2, 4, 8]
take.(3, (1..).lazy.map { |x| x * x })                 # => [1, 4, 9]
```

`LazyList` is Enumerable and offers `head`, `tail`, `take(n)`, `empty?`, plus
constructors `iterate`, `repeat`, `range`, `generate` and `empty`.

### Clause bodies and `self`

The definition block runs with `self` set to the clause builder. Method calls
the builder does not understand are forwarded to the object that owns the
block, so helper methods remain callable. Instance variables are not visible;
take the builder as a parameter when you need them:

```ruby
HaskellMatch.fn(:f) { |m| m.on("Just x") { |x| x + @offset }; m.on("Nothing") { @offset } }
```

Methods defined with `hdef` run their bodies with `self` set to the receiver
(see below).

## Three ways to match

### `HaskellMatch.fn` — compiled functions

```ruby
zip = HaskellMatch.fn(:zip) do
  on("(x:xs)", "(y:ys)") { |x, xs, y, ys| [[x, y]] + zip.(xs, ys) }
  on("_", "_")           { [] }
end
zip.([1, 2, 3], %i[a b])     # => [[1, :a], [2, :b]]
zip.arity                    # => 2
zip.bindings                 # => [["x", "xs", "y", "ys"], []]
zip.to_proc, zip.curry, zip[a, b], zip === a   # Proc-like protocol
zip.select(args...)          # => [clause_index, [bound values]] or nil
zip.decision_tree            # dump of the compiled tree
```

Options: `exhaustive:` and `overlapping:` accept `:error` (default), `:warn`
or `:ignore` (`true`/`false` work too), per function or globally through
`HaskellMatch.exhaustive = :warn`. A non-exhaustive function compiled with
`exhaustive: false` raises `HaskellMatch::MatchError` when no clause matches.

### `HaskellMatch.case_of` — `case ... of` expressions

```ruby
HaskellMatch.case_of(value) do
  on("Just x", guard: ->(x) { x > 10 }) { |x| "big #{x}" }
  on("Just x")  { |x| "just #{x}" }
  on("Nothing") { "nothing" }
end
```

Several scrutinees may be given: `case_of(a, b) { on("(x:_)", "True") { ... } }`.
The decision tree for each distinct set of patterns is compiled once and
cached; the blocks are collected on each evaluation, so `fn` is the fast path
for hot code.

### `HaskellMatch.pattern` — single patterns

```ruby
p = HaskellMatch.pattern("Just (x:rest)")      # also HaskellMatch["..."]
p.match(Just.new([1, 2, 3]))   # => { x: 1, rest: [2, 3] }
p.match(Nothing)               # => nil
p.match!(Nothing)              # raises MatchError
p === Just.new([1])            # => true, so it works in case/when
p.irrefutable?                 # => false
```

### Methods: `hdef`

```ruby
class Account
  extend HaskellMatch::DSL      # fn, case_of, pattern, data, hdef
  include Maybe

  hdef :apply do
    on("Nothing")   { self }
    on("Just amt", guard: ->(amt) { amt <= limit }) { |amt| Account.new(balance + amt) }
    on("Just amt")  { |amt| raise ArgumentError, "over limit: #{amt}" }
  end
end
```

## Errors

All errors inherit from `HaskellMatch::Error`.

Definition time (`HaskellMatch::CompileError`): `PatternSyntaxError`,
`UnknownConstructorError`, `ArityError` (constructor applied to the wrong
number of patterns), `PatternTypeError` (one position matched against two
types), `DuplicateVariableError`, `FieldError`, `DataDeclarationError`,
`ClauseArityError`, `NonExhaustiveError` (`#missing` lists the witnesses),
`RedundantClauseError` (`#clauses` lists the indices), `DefinitionError`.

Match time (`HaskellMatch::MatchError`): `MatchError` itself for a partial
function with no matching clause, `TypeMismatchError` when a value is of a
type no pattern of the function can accept (`expected a value of type Maybe
but got 5 (Integer)`), `IrrefutablePatternError` when a `~` pattern fails to
destructure. `HaskellMatch::StackOverflowError` is raised past
`HaskellMatch.max_depth`. Wrong argument counts raise `ArgumentError`.

Exceptions raised in bodies and guards propagate unchanged, with their
backtraces; `throw`, `return` and `next` behave as in any block.

## Concurrency

* **Threads**: safe. Native code runs under the GVL, the type registry is
  behind a mutex that is never held while calling back into Ruby, compiled
  functions are immutable, and per-call state lives on the stack.
* **Fibers**: safe; no per-thread or per-fiber state is kept natively.
* **Ractors**: the extension is declared Ractor-safe and data values are
  shareable. A function is shareable when defined with `ractor: true`, which
  makes its clause procs shareable (their `self` becomes an inert frozen
  object and any local variables they capture must already be shareable) and
  passes the function through `Ractor.make_shareable`:

  ```ruby
  f = HaskellMatch.fn(:f, ractor: true) { on("Just x") { |x| x }; on("Nothing") { 0 } }
  Ractor.new(f) { |g| g.(Just.new(1)) }.take   # => 1
  ```

  Inside a shareable function, recursion goes through the function's name or
  `recur` (the body's `self` is a module that knows the finished function).
  Do not also keep the function in a local variable of the same name: the
  variable would shadow the name, and `make_shareable` fixes a captured
  local's value (still `nil` at that point).

## Performance

`rake bench` compares against hand-written Ruby (Ruby 3.3, x86-64, one run;
numbers are ns per call):

| benchmark                                   | haskell_match | Ruby `case/in` | hand-written Ruby |
|---------------------------------------------|--------------:|---------------:|------------------:|
| `from_maybe` (2 args, constructor switch)   |           201 |            238 |     97 (`case/when`) |
| `area` (3 constructors, nested arithmetic)  |           270 |            356 |                   |
| `length` of a 20-element list (recursive)   |          7850 |           4686 |  2092 (slices)    |
| `fib(20)` via literals `0`, `1`, `n`        |     3 866 000 |                | 1 216 000 (`if`)  |
| `pattern.match` (bindings hash)             |           548 |                |                   |
| `case_of` (blocks collected per call)       |        14 600 |                |                   |

The matcher itself is cheap; what remains per call is one native method
dispatch plus one Ruby block invocation for the body, which inline Ruby code
does not pay. Recursive list functions additionally allocate one shared-slice
Array per `(x:xs)` tail that is bound.

## How it works

`ext/haskell_match` is a Rust crate with a pure core and a thin Ruby layer:

* `core::lexer`, `core::parser` — Haskell pattern and `data` declaration syntax.
* `core::types`, `core::resolve` — the type environment and name resolution
  (arity checks, record fields, variable numbering).
* `core::typecheck` — every position across all clauses must have one type.
* `core::exhaust` — Maranget's usefulness algorithm: redundancy and the list
  of unmatched patterns, including literal positions (`p1 where p1 is not one
  of {0, 1}`).
* `core::tree` — compilation to a decision tree over numbered value slots.
* `ruby::runtime` — evaluation over Ruby values, including Strings and lazy
  lists viewed as lists; `ruby` — the `Native` module, the trampoline that
  handles `tail`/`defer` markers and the Fiber-segmented recursion.
* `lib/haskell_match/pattern_ast.rb` — the in-place pattern syntax, rendered
  to Haskell text; `lazy_list.rb` — memoised lazy lists.

The pure core has its own `cargo test` suite; the Ruby suite covers the
public API, GC stress, threads and Ractors.

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or
  http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
