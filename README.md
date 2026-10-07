# haskell_match

Haskell's pattern matching, brought to Ruby in full: algebraic data types,
clauses that destructure their arguments, and a compiler that refuses to build
a function with a hole in it. The matcher is written in Rust (via
[Rutie](https://github.com/danielpclark/rutie)), compiles each function once
into a decision tree, and runs it directly over Ruby values.

```ruby
require "haskell_match"

HaskellMatch.data "Shape = Circle Double | Rect Double Double | Triangle Double Double Double"
include Shape

area = HaskellMatch.fn(:area) do
  on("Circle r")       { |r| 3.14159 * r * r }
  on("Rect w h")       { |w, h| w * h }
  on(Triangle(a, b, c)) do |a, b, c|
    s = (a + b + c) / 2.0
    Math.sqrt(s * (s - a) * (s - b) * (s - c))
  end
end

area.(Rect.new(2, 3))          # => 6
area.(Triangle[3, 4, 5])       # => 6.0
```

Two of those clauses are Haskell written in a string; the third is the same
pattern written as Ruby. Both forms build the same decision tree, and you can
use whichever reads better, line by line.

## Why every path must be accounted for

In most Ruby code, the case you forgot is found by a user. A `case` with no
`else` silently returns `nil`; an `if` chain that misses a branch falls
through; a `Hash#fetch` without a default raises in production at three in the
morning. The fix is always the same, and always late: add the branch you did
not think of.

Haskell inverts this. A function defined by patterns is a *claim* about the
shape of its input, and the compiler checks the claim: if the clauses do not
cover every value the type allows, the program does not compile, and the
message tells you exactly which values fell through. haskell_match brings that
discipline to Ruby at definition time:

```ruby
HaskellMatch.fn(:area) do
  on("Circle r") { |r| 3.14159 * r * r }
  on("Rect w h") { |w, h| w * h }
end
# HaskellMatch::NonExhaustiveError:
# Pattern match(es) are non-exhaustive
# In an equation for 'area':
#     Patterns not matched:
#         Triangle _ _ _
```

This changes how code evolves. Add a constructor to a type (`| Hexagon
Double`) and every function that matches on that type fails to load, each one
pointing at the exact case to write. There is no grep for call sites, no
"should be fine", no test suite hoping to cover the new branch: the compiler
has already enumerated the paths and found the missing one. Teams that adopt
this stop writing defensive `else raise "unreachable"` branches, because
unreachable is now something the compiler proves rather than something a
comment asserts.

The same analysis catches the opposite mistake, a clause that can never run:

```ruby
HaskellMatch.data "Maybe a = Nothing | Just a"
include Maybe

HaskellMatch.fn(:describe) do
  on("_")      { "something" }
  on("Just x") { |x| "just #{x}" }
end
# HaskellMatch::RedundantClauseError:
# Pattern match is redundant
# In an equation for 'describe':
#     describe Just x = ... (example.rb:3)
```

Redundant clauses are dead code with a story: usually an earlier clause grew
broader than intended, or two people each handled a case. Either way the
compiler found the contradiction in the reasoning before it became a bug.

The check is precise, not merely cautious. Nested patterns, literals, lists,
tuples and records are all enumerated, and the witnesses are concrete:

```ruby
HaskellMatch.data "Tree a = Leaf | Node (Tree a) a (Tree a)"
include Tree

HaskellMatch.fn(:depth) do
  on("Leaf")                   { 0 }
  on("Node Leaf _ Leaf")       { 1 }
  on("Node (Node _ _ _) _ _")  { |*| :deep }
end
# Patterns not matched:
#     Node Leaf _ (Node _ _ _)
```

Haskell's type checker also rejects a function that matches a `Maybe` in one
clause and a list in another. Ruby has no static types, so haskell_match
checks what it can at definition time (all clauses must agree on the shape of
each position) and defers the rest to the call: a value that *no* clause could
accept raises `TypeMismatchError`, while `_` and variables accept anything,
exactly as Haskell's wildcard does.

## A tour, in both dialects

Everything below mixes the two ways of writing a pattern. A String given to
`on` is Haskell syntax; anything else is the pattern written in place, where
bare names are variables, `_` is the wildcard, `[x, *xs]` is `(x:xs)`, and
`Just(x)` or `Just[x]` applies a constructor.

### Data types and constructors

```ruby
HaskellMatch.data "Either a b = Left a | Right b"
HaskellMatch.data "Contact = Person { name :: String, age :: Int }"
include Either
include Contact

Just.new(1)                          # => Just 1
Just[Just[Nothing]]                  # => Just (Just Nothing)
Person.new(name: "Ann", age: 30)     # => Person {name = "Ann", age = 30}
[1, 2].map(&Just)                    # => [Just 1, Just 2]
Nothing.frozen?                      # => true
```

Constructors are frozen `Data` values: they compare by value, hash, print as
Haskell would, and work with Ruby's own `case/in` too.

### Maybe and Either, the everyday cases

```ruby
from_maybe = HaskellMatch.fn(:from_maybe) do
  on("d", "Nothing") { |d| d }
  on(_, Just(x))     { |x| x }
end
from_maybe.(0, Just.new(5))          # => 5
from_maybe.(0, Nothing)              # => 0

either = HaskellMatch.fn(:either) do
  on("Left e")  { |e| "error: #{e}" }
  on(Right(v))  { |v| "ok: #{v}" }
end
either.(Left.new("boom"))            # => "error: boom"
either.(Right[42])                   # => "ok: 42"
```

### Lists, strings and recursion

A Ruby Array is a Haskell list, and so is a Ruby String (`String = [Char]`):

```ruby
length = HaskellMatch.fn(:length) do
  on("[]")      { 0 }
  on([_, *xs])  { |xs| 1 + length.(xs) }
end
length.([1, 2, 3])                   # => 3
length.("haskell")                   # => 7

zip = HaskellMatch.fn(:zip) do
  on("(x:xs)", [y, *ys]) { |x, xs, y, ys| [[x, y]] + zip.(xs, ys) }
  on("_", "_")           { [] }
end
zip.([1, 2, 3], %w[a b])             # => [[1, "a"], [2, "b"]]

greeting = HaskellMatch.fn(:greeting) do
  on('""')             { "Hello, stranger" }
  on("('A':_)")        { "Hello, A-person" }
  on([c, *_])          { |c| "Hello, #{c.upcase}-person" }
end
greeting.("")                        # => "Hello, stranger"
greeting.("Ann")                     # => "Hello, A-person"
greeting.("bob")                     # => "Hello, B-person"
```

Bound tails share storage with the original (a copy-on-write slice), so
recursing down a list does not copy it.

### Guards, as-patterns and nesting

```ruby
classify = HaskellMatch.fn(:classify) do
  on(Just(x), guard: ->(x) { x.negative? })  { :negative }
  on("Just 0")                               { :zero }
  on(Just(x), where: ->(x) { x.even? })      { :even }
  on("Just _")                               { :odd }
  on(Nothing)                                { :none }
end
[Just[-1], Just[0], Just[2], Just[3], Nothing].map(&classify)
# => [:negative, :zero, :even, :odd, :none]

dedupe = HaskellMatch.fn(:dedupe) do
  on("(x:rest@(y:_))", guard: ->(x, y) { x == y }) { |rest| dedupe.(rest) }
  on([x, *rest])                                    { |x, rest| [x] + dedupe.(rest) }
  on([])                                            { [] }
end
dedupe.([1, 1, 2, 3, 3, 3, 4])      # => [1, 2, 3, 4]

flatten_maybe = HaskellMatch.fn(:flatten_maybe) do
  on("Just (Just x)") { |x| Just[x] }
  on(Just(Nothing))   { Nothing }
  on(Nothing)         { Nothing }
end
flatten_maybe.(Just[Just[7]])        # => Just 7
```

A guarded clause may fall through, so (as in GHC) it does not count towards
coverage; `otherwise` does.

### Records and tuples

```ruby
can_vote = HaskellMatch.fn(:can_vote) do
  on("Person { age = a }", guard: ->(a) { a >= 18 }) { true }
  on(Person(**_))                                    { false }
end
can_vote.(Person.new("Ann", 30))     # => true

introduce = HaskellMatch.fn(:introduce) do
  on(Person(name: n, age: a)) { |n, a| "#{n} is #{a}" }
end
introduce.(Person.new("Bob", 7))     # => "Bob is 7"

swap = HaskellMatch.fn(:swap) { on("(a, b)") { |a, b| [b, a] } }
swap.([1, 2])                        # => [2, 1]

dist = HaskellMatch.fn(:dist) { on(tuple(x1, y1), tuple(x2, y2)) { |x1, y1, x2, y2| Math.hypot(x2 - x1, y2 - y1) } }
dist.([0, 0], [3, 4])                # => 5.0
```

Tuples are Arrays of a fixed length, lists are Arrays of any length; the
compiler keeps the two apart just as Haskell does.

### Expressions, patterns as objects, and methods

```ruby
# case ... of
HaskellMatch.case_of(Just[3]) do
  on("Just x", guard: ->(x) { x > 10 }) { |x| "big #{x}" }
  on(Just(x))  { |x| "just #{x}" }
  on(Nothing)  { "nothing" }
end                                  # => "just 3"

# a pattern on its own, usable in case/when
head = HaskellMatch.pattern { Just([x, *_]) }
head.match(Just[[9, 8]])             # => {:x=>9}
head === Just[[]]                    # => false

# methods defined by clauses, with access to self
class Account
  extend HaskellMatch::DSL
  include Maybe

  attr_reader :balance
  def initialize(balance) = @balance = balance
  def limit = 100

  hdef :deposit do
    on(Nothing)     { self }
    on(Just(amt), guard: ->(amt) { amt <= limit }) { |amt| Account.new(balance + amt) }
    on("Just amt")  { |amt| raise ArgumentError, "over limit: #{amt}" }
  end
end
Account.new(10).deposit(Just[50]).balance   # => 60
```

### Bindings your way

Name your block parameters after the pattern's variables and take any subset
in any order; keywords work too:

```ruby
f = HaskellMatch.fn(:f) do
  on("(x:xs)") { |xs| xs }            # by name
  on("[]")     { [] }
end
f.([1, 2, 3])                        # => [2, 3]

g = HaskellMatch.fn(:g) { on([x, *xs]) { |x:, xs:| { x: x, xs: xs } }; on([]) { {} } }
g.([1, 2])                           # => {:x=>1, :xs=>[2]}
```

## Recursion without limits, and laziness

Haskell programs loop by recursing and process infinite data by being lazy.
Ruby's VM gives a fixed stack to each thread and evaluates eagerly, so a
faithful port has to supply both. haskell_match does, in three complementary
ways.

### Plain recursion goes as deep as memory allows

```ruby
count = HaskellMatch.fn(:count) do
  on([])        { 0 }
  on([_, *xs])  { |xs| 1 + count.(xs) }
end
count.((1..200_000).to_a)            # => 200000
```

A Ruby lambda written the same way dies with `SystemStackError` around 10,000
levels. Here the native call runs every hundredth nested body on a fresh
Fiber, chaining their stacks the way GHC grows its own, so the depth limit
becomes memory rather than a fixed buffer. You write the obvious code and it
works; the cost is about 1.6 KB per level while the recursion is pending, and
`HaskellMatch.max_depth` (250,000 by default) turns a runaway recursion into
a clear `StackOverflowError` instead of a swallowed machine. A function that
is *meant* to recurse deep can be defined with `deep: true`: its body is then
invoked from Ruby rather than from the native matcher, which costs about one
extra Ruby frame per call but brings a level down to about 100 bytes and
makes garbage collection at depth six times cheaper.

### Tail calls run in constant space

```ruby
sum = HaskellMatch.fn(:sum) do
  on(acc, [])        { |acc| acc }
  on(acc, [x, *xs])  { |acc, x, xs| sum.tail(acc + x, xs) }
end
sum.(0, (1..1_000_000).to_a)         # => 500000500000

collatz_steps = HaskellMatch.fn(:collatz_steps) do
  on(1, n)                                { |n| n }
  on(k, n, guard: ->(k) { k.even? })      { |k, n| collatz_steps.tail(k / 2, n + 1) }
  on(k, n)                                { |k, n| collatz_steps.tail(3 * k + 1, n + 1) }
end
collatz_steps.(27, 0)                # => 111
```

`f.tail(args)` is Haskell's tail call made explicit: the native loop replaces
the arguments and matches again on the same frame. Any loop a Haskell program
would write as an accumulator, a fold, a state machine or a server loop runs
this way in O(1) space, including mutual recursion between functions, with
nothing counted against the depth limit.

### Deferred calls keep deep recursion cheap

```ruby
length = HaskellMatch.fn(:length) do
  on([])        { 0 }
  on([_, *xs])  { |xs| length.defer(xs) { |n| 1 + n } }    # 1 + length xs
end
length.((1..2_000_000).to_a)         # => 2000000
```

`defer` names the continuation that plain recursion leaves implicit. The
pending blocks live on a heap stack owned by the native call, about 200 bytes
each, chunked so Ruby's generational GC never rescans the whole stack. Use it
when a non-tail recursion is known to go very deep and memory matters.

### Lazy lists make infinite data ordinary

```ruby
take = HaskellMatch.fn(:take) do
  on(0, _)          { [] }
  on(_, [])         { [] }
  on(n, [x, *xs])   { |n, x, xs| [x] + take.(n - 1, xs) }
end

naturals = HaskellMatch.lazy(1..)
take.(5, naturals)                               # => [1, 2, 3, 4, 5]

powers = HaskellMatch::LazyList.iterate(1) { |x| x * 2 }
take.(8, powers)                                 # => [1, 2, 4, 8, 16, 32, 64, 128]

fibs = HaskellMatch.lazy(Enumerator.new { |y| a, b = 0, 1; loop { y << a; a, b = b, a + b } })
take.(10, fibs)                                  # => [0, 1, 1, 2, 3, 5, 8, 13, 21, 34]

# any Enumerator is a list; Ruby's lazy pipelines compose with the patterns
take.(3, (1..).lazy.select(&:even?).map { |x| x * x })   # => [4, 16, 36]
```

A `LazyList` is a memoised cons list: `(x:xs)` forces one cell, binds `xs` to
the rest still unevaluated, and every forced cell is computed once and shared
by all consumers, which is exactly Haskell's evaluation model for lists. The
advantages carry over intact. Producers and consumers are written separately
and composed; a generator never needs to know how much of it will be used;
`take.(n, expensive_stream)` does `n` units of work and no more; and the same
`take` serves finite Arrays, Strings, lazy lists and Ruby Enumerators without
a line changing, because they are all one type to the pattern compiler.

Together these give Ruby the two things Haskell relies on for "infinite"
programs: loops that do not consume stack, and data that does not have to
exist before it is asked for.

## Or simply write Haskell

Everything above is Haskell's pattern matching with Ruby expressions in the
clause bodies. When a function is clearer in Haskell itself, write it in
Haskell. `HaskellMatch.haskell` compiles a Haskell 2010 subset (data
declarations, equations, guards, `where`, `let`, `case`, lambdas, sections,
list comprehensions, ranges, and a lazy Prelude) into methods on a Ruby
module, with the same exhaustiveness and redundancy checks as everything
else in this library:

```ruby
Shapes = HaskellMatch.haskell(<<~HS)
  data Shape = Circle Double | Rect Double Double

  area :: Shape -> Double
  area (Circle r) = 3 * r * r
  area (Rect w h) = w * h

  describe :: Shape -> String
  describe s
    | a > 10 = "big " ++ kind
    | otherwise = "small " ++ kind
    where
      a = area s
      kind = case s of
        Circle _ -> "circle"
        Rect w h | w == h -> "square"
                 | otherwise -> "rectangle"

  totalArea :: [Shape] -> Double
  totalArea = sum . map area
HS

shapes = [Shapes::Circle.new(1.0), Shapes::Rect.new(2.0, 2.0), Shapes::Rect.new(3.0, 5.0)]
shapes.map { |s| Shapes.describe(s) }  # => ["small circle", "small square", "big rectangle"]
Shapes.total_area(shapes)              # => 22.0
```

Haskell names arrive as Ruby methods (`totalArea` is also `total_area`),
constructors as constants, and the recursion and laziness machinery is the
one described above: `where` helpers that call themselves in tail position
run in constant space, and a list built with `:` is as lazy as Haskell's, so
the classic infinite definitions work unchanged.

```ruby
Nums = HaskellMatch.haskell(<<~HS)
  primes :: [Int]
  primes = sieve [2..]
    where
      sieve [] = []
      sieve (p:xs) = p : sieve [x | x <- xs, x `mod` p /= 0]

  fibs :: [Integer]
  fibs = 0 : 1 : zipWith (+) fibs (tail fibs)

  sumTo :: Int -> Int
  sumTo n = go n 0
    where
      go 0 acc = acc
      go k acc = go (k - 1) (acc + k)
HS

Nums.primes.take(8)    # => [2, 3, 5, 7, 11, 13, 17, 19]
Nums.fibs.take(10)     # => [0, 1, 1, 2, 3, 5, 8, 13, 21, 34]
Nums.sum_to(1_000_000) # => 500000500000
```

The two languages call each other freely. A name the Haskell does not define
is a Ruby method of the host module, so Haskell can lean on Ruby for
formatting, I/O or anything else; and Ruby can match on the module's types
with the module's own `fn`, `case_of` and `pattern`, in either pattern
dialect:

```ruby
module Report
  def self.money(x)
    format("$%.2f", x)
  end

  extend HaskellMatch::Haskell
  haskell <<~HS
    data Line = Line String Double Int

    total :: [Line] -> Double
    total ls = sum [price * fromIntegral qty | Line _ price qty <- ls]

    summary :: [Line] -> String
    summary [] = "nothing ordered"
    summary ls = show (length ls) ++ " lines, " ++ money (total ls)
  HS
end

order = [Report::Line.new("tea", 2.5, 2), Report::Line.new("cake", 4.0, 1)]
Report.summary(order)   # => "2 lines, $9.00"
Report.summary([])      # => "nothing ordered"

label = Report.fn(:label) do
  on(Line(name, _, 1))   { |name| name }
  on("Line name _ qty")  { |name, qty| "#{qty} x #{name}" }
end
order.map { |l| label.(l) }   # => ["2 x tea", "cake"]
```

And a Haskell file is just a Haskell file. `HaskellMatch.require` finds
`name.hs` on the load path (or takes a path) and defines a constant named
after its `module` header; `HaskellMatch.load` returns an anonymous module.
No templating and no interpolation: the file is the Haskell 2010 subset
described in [Inline Haskell and `.hs` files](#inline-haskell-and-hs-files).

```ruby
HaskellMatch.require "examples/geometry"   # examples/geometry.hs: `module Geometry where ...`
Geometry.describe(Geometry::Triangle.new(3.0, 4.0, 5.0))   # => "small triangle"
```

## At a glance

* **Haskell pattern syntax**, quoted or written in place: constructors,
  literals, variables, wildcards, lists, tuples, as-patterns `all@(x:_)`,
  lazy patterns `~p`, bang patterns `!x`, record patterns `Person { name = n, .. }`,
  guards and `otherwise`, with both forms freely mixed.
* **Algebraic data types** declared with Haskell `data` syntax; constructors
  are frozen `Data` values with Haskell-style `inspect`.
* **Every logical path accounted for**: non-exhaustive and redundant clauses
  are definition-time errors with GHC-style messages; positions whose
  patterns disagree in type are a compile error; `_` and variables match
  anything, and a value no pattern can accept raises `TypeMismatchError`.
* **Strings are lists of characters**, as in Haskell; lazy lists and
  Enumerators are lists too.
* **Recursion without Ruby's stack limit**: chained Fiber stacks, constant-
  space tail calls, deferred continuations, and a depth guard.
* **Fast**: one Maranget-style decision tree per function, matching in Rust
  over Ruby `VALUE`s with no allocation until a clause is chosen, and list
  tails as shared slices.
* **Thread-, fiber- and Ractor-safe**, with `ractor: true` producing
  shareable functions.
* **Haskell itself**, inline or from `.hs` files: a Haskell 2010 subset
  compiled to Ruby methods, with a lazy Prelude and two-way interop.

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
`Person.data_type`. The type module forwards `new`, `[]` and `call` to a
same-named constructor, so `Person.new("Al", 3)` builds a person either way.

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
Tail calls do not count towards the depth. The section "Deep and infinite
recursion: expert notes" below gives the measured costs and the semantics at
segment boundaries.

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
`HaskellMatch.exhaustive = :warn`; `deep: true` selects deep mode (see the
recursion notes); `ractor: true` makes the function Ractor-shareable. A non-exhaustive function compiled with
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

## Ruby values, Haskell discipline

Everything above works on values you declare with `HaskellMatch.data`. This
section is about the rest of Ruby: Hashes, the `Data` and `Struct` classes
you already have, and the conveniences Haskell programmers expect.

### Hash patterns

A Ruby Hash literal is a Hash pattern (quoted: `{name = n, "key" = p}`).
It matches a Hash that has every listed key (Symbol or String), with each
value matched by its sub-pattern; other keys are ignored, like fields left
out of a record pattern. A key whose value is `nil` counts as present.

```ruby
greet = HaskellMatch.fn(:greet) do
  on({ name: n, title: t }) { |n, t| "#{t} #{n}" }
  on({ name: n })           { |n| "hi #{n}" }
  on({})                    { "anonymous" }          # {} matches every Hash
end
greet.(name: "Al", title: "Dr")   # => "Dr Al"
greet.(name: "Al", age: 3)        # => "hi Al"
```

Exhaustiveness and redundancy are checked as for everything else: without
a `{}` or `_` clause the checker reports `p1 where p1 is a Hash without the
key name`; a clause needing a superset of an earlier clause's keys is
redundant; a Hash column cannot be mixed with constructors or literals.
Values nest (`{ user: { id: i } }`, `{ k: Just(x) }`), and a non-Hash
argument with no wildcard clause is a `TypeMismatchError`.

### Ruby classes as constructors

A `Data` or `Struct` class visible from the definition needs no
declaration: naming it in a pattern makes it a type with that single
constructor, so one clause covers it and its members are its fields (record
patterns included).

```ruby
Point = Data.define(:x, :y)
norm = HaskellMatch.fn(:norm) { on(Point(x, y)) { |x, y| Math.hypot(x, y) } }
norm.(Point.new(3, 4))                                           # => 5.0
HaskellMatch.pattern("Point { x = a }").match(Point.new(1, 2))   # => {a: 1}
```

For several classes that together form one closed type (a sealed
hierarchy), `HaskellMatch.sealed` lists them. `Data` and `Struct` classes
bring their own field names; for any other class, name the reader methods
that are its fields.

```ruby
Circle = Data.define(:r)
Rect   = Data.define(:w, :h)
class Tri
  attr_reader :a, :b, :c
  def initialize(a, b, c) = (@a, @b, @c = a, b, c)
end

Shape = HaskellMatch.sealed(:Shape, Circle, Rect, Tri => %i[a b c])

area = HaskellMatch.fn(:area) do
  on(Circle(r))    { |r| Math::PI * r * r }
  on("Rect w h")   { |w, h| w * h }
  # on("Tri a b c") missing:
end
# HaskellMatch::NonExhaustiveError: Patterns not matched: Tri _ _ _
```

The classes themselves are untouched apart from gaining a few readers
(`constructor_name`, `field_names`, `data_type`). Instances are identified
by exact class, so list the leaf classes of a hierarchy. The returned type
module works like any other (`Shape === value`, `Shape.constructors`) and
is defined as a constant only when you pass `under:`.

### `deriving (Ord, Enum, Bounded)`

`Eq` and `Show` always hold: constructor values compare structurally and
`inspect` prints Haskell. Deriving the other standard classes adds:

```ruby
HaskellMatch.data "Color = Red | Green | Blue deriving (Eq, Ord, Enum, Bounded, Show)"
include Color

Red < Blue                                 # => true   (Ord: constructor order, then fields)
[Blue, Red, Green].sort                    # => [Red, Green, Blue]
Red.succ                                   # => Green  (Enum; Blue.succ raises, as in GHC)
(Red..Blue).to_a                           # => [Red, Green, Blue]
Color.enum_from(Green)                     # => [Green, Blue]
Color.enum_from_then_to(Red, Blue, Blue)   # => [Red, Blue]
Color.min_bound                            # => Red    (Bounded)
Green.from_enum                            # => 1
```

`Ord` works for any type (`Pt 1 2 < Pt 1 3`); `Enum` and `Bounded` need an
enumeration (every constructor nullary), as GHC requires. The Hash form
takes `deriving: %i[Ord Enum]`. Unsupported class names are an error, and
nothing is registered when a derivation fails.

### Checked field types

The types written in a declaration are documentation by default. With
`check_types: true` (or `HaskellMatch.check_field_types = true` for all
declarations) the constructor verifies each argument and raises
`FieldTypeError` otherwise:

```ruby
HaskellMatch.data "Person = Person { name :: String, age :: Int, boss :: Maybe Person }",
                  check_types: true
Person.new("Al", "3", Nothing)
# HaskellMatch::FieldTypeError: Person: field 'age' expects Int, got "3" (String)
```

`Int`/`Integer` take Integers; `Double`/`Float` any Numeric; `String` a
String or list of characters; `Char` a one-character String; `Bool`
true/false; `[a]` any list (Array, String, LazyList, Enumerator);
`(a, b)` an Array of that size, elementwise; `a -> b` anything callable; a
declared type (`Maybe Person`, `Shape`) a value of that type; any other
capitalised name a Ruby class or module of that name when one exists
(`Time`, `Hash`, `MyApp::Money`); type variables anything.

### `where` helpers

A local function inside a definition, like a Haskell `where` binding. It is
a full `Function` (checked, with `tail` and `defer`), compiled with the
enclosing function's options unless you override them, and reachable by
name from every clause body and from the other helpers.

```ruby
sum_to = HaskellMatch.fn(:sum_to) do
  on(n) { |n| go.(n, 0) }
  where :go do
    on(0, acc) { |acc| acc }
    on(k, acc) { |k, acc| go.tail(k - 1, acc + k) }
  end
end
sum_to.(1_000_000)        # => 500000500000
sum_to.helpers            # => {"go" => #<HaskellMatch::Function go/2>}
```

Helpers do not see the enclosing clause's variables (pass them as
arguments, as the example does), and in `ractor: true` mode they cannot
call back into the enclosing function by name.

### Composition

`Function#>>` and `#<<` compose like `Proc`'s: `(f >> g).(x)` is
`g.(f.(x))` (Haskell's `g . f`) and `(f << g).(x)` is `f.(g.(x))`.

### Errors point at the problem

Pattern syntax errors show the pattern with a caret under the offending
column, and Haskell syntax errors show the offending line of the Ruby or
`.hs` file:

```
HaskellMatch::PatternSyntaxError: clause 1: expected ')' but reached end of pattern
    Just (x
          ^
```

## Inline Haskell and `.hs` files

`HaskellMatch.haskell(source)` compiles Haskell source into a module and
returns it; `into: SomeModule` compiles into an existing one, and inside a
module body `extend HaskellMatch::Haskell` gives a `haskell` method that
does the same. `HaskellMatch.load(path)` compiles a file into a fresh
module; `HaskellMatch.require(name)` finds `name.hs` on `$LOAD_PATH` (or
takes a path), compiles it once, and defines a constant for it named after
the file's `module` header (`module Data.Tree where` becomes `Data::Tree`)
or, without one, the camel-cased file name.

```ruby
Geometry = HaskellMatch.load("examples/geometry.hs")
HaskellMatch.require "lists"               # lists.hs somewhere on $LOAD_PATH -> Lists
HaskellMatch.require "lib/hs/tree", under: MyApp   # -> MyApp::Tree (or its module header)
```

Write the source in a heredoc; use a quoted heredoc (`<<~'HS'`) when the
Haskell contains backslashes (lambdas) so Ruby leaves them alone.

### The supported language

The front end is a Haskell 2010 parser with the layout rule, so ordinary
Haskell formatting works. Supported:

* `data` and `newtype` declarations, positional, infix (`data V = Double
  :| Double`) or with record fields (whose names are selector functions,
  with `P { f = e }` construction and `p { f = e }` update), with
  `deriving` (`Eq` and `Show` always hold; `Ord`, `Enum` and `Bounded` work
  as described under [deriving](#deriving-ord-enum-bounded), so `succ c`,
  `[Red ..]`, `minBound`-style code runs); `type` synonyms and signatures
  (accepted and ignored: Ruby is the type system here).
* `module M (exports) where` headers and `import` declarations: see
  [Modules](#modules-imports-and-exports) below.
* User-defined operators with `infixl`/`infixr`/`infix` fixity
  declarations, in prefix form (`(<+>) a b = ...`), infix form
  (`a <+> b = ...`) or with backticks (``x `cons` xs = ...``), at top level
  or in `where`/`let`; operators and backticked functions in sections, as
  values (`(<+>)`) and from Ruby (`Mod.send(:"<+>", a, b)`,
  `Mod.haskell_function("<+>")`).
* Function equations with any patterns this library supports (constructors,
  infix constructors, literals, negative literals, characters, strings,
  lists, tuples, `_`, as-patterns, lazy and bang patterns, records), guards
  with `otherwise`, pattern guards (`| Just v <- lookup k m, v > 0 = ...`)
  and `let` guards, `where` bindings (functions, values and pattern
  bindings, nested arbitrarily), `let ... in`, `case ... of` with guards,
  `if/then/else`.
* Expressions: application and partial application, operators with the
  Prelude's fixities, backtick operators, sections (`(*2)`, `` (`div` 2) ``,
  `subtract 1`), operator values (`(+)`), `$`, `.`, lambdas (including
  pattern lambdas), tuples, list literals, ranges (`[1..n]`, `[1,3..]`,
  `[0..]`), list comprehensions with generators, guards and `let`.
* The small syntax extensions GHC users reach for without thinking:
  `MultiWayIf` (`if | c1 -> e1 | c2 -> e2`), `LambdaCase` (`\case`),
  `TupleSections` (`(,x)`, `(1,,3)`), `NamedFieldPuns` in construction.
* Literals: decimal, hexadecimal, octal and binary integers, exponent
  floats, `_` digit separators, and every Haskell character escape
  (`\n`, `\x41`, `\o101`, `\65`, `\SOH`, `\^A`, `\&`, string gaps).
* Names: `camelCase` functions get a `snake_case` alias; a trailing prime
  becomes `_prime` (`foldl'` is callable as `foldl_prime`).
* Top-level values (`primes = ...`) become memoised methods; a value of
  function type can still be called with arguments from Ruby
  (`Mod.from_list([1, 2])` when `fromList = foldr insert Leaf`).

Out of scope, by design: type classes (`class`/`instance`), `do` notation
and monads, and anything needing type inference. They are rejected with a
clear message.

### Modules, imports and exports

A compiled module can import another. Functions and values of the imported
module become callable (the compiler uses them with their real arity, tail
calls included), its data types join the importing module's type scope so
their constructors work in patterns with full exhaustiveness checking, and
the constructors become constants of the importing module.

```haskell
module Physics where
import Vectors                         -- Vectors.hs on $LOAD_PATH, or the constant Vectors
import Vectors (Vec(..), norm)         -- only these
import Vectors hiding (hidden)         -- all but these
import qualified Data.Map as M         -- qualification is accepted and ignored
import Math (hypot)                    -- a plain Ruby module: its singleton methods
```

A module name resolves to an existing constant (`Data.Tree` is
`Data::Tree`) or to a file on `$LOAD_PATH` (`Data/Tree.hs`, `Data.Tree.hs`
or `data/tree.hs`), loaded once with `HaskellMatch.require`. The standard
library modules (`Data.List`, `Data.Char`, `Data.Maybe`, `Control.Monad`,
...) are the Prelude: importing one only checks that the named items exist.

An export list (`module Vectors (Vec(..), (<+>), norm) where`) limits what
importers see; `Mod.haskell_exports` returns it. Everything stays callable
from Ruby regardless. Compiled code refers to names unqualified, so two
qualifications of the same name are not distinguished.

### Semantics worth knowing

* **Strictness.** Compiled code is strict (it is Ruby), with one deliberate
  exception: the tail of `x : e` is deferred when `e` is a computation
  rather than a variable or literal. That is exactly what makes
  `p : sieve xs` and `fibs = 0 : 1 : zipWith (+) fibs (tail fibs)` work. A
  list built that way is a `LazyList` (`to_a` materialises it, `==`
  compares elements); a list built from a variable tail (`toUpper c : cs`)
  keeps its input's type, so Strings stay Strings and Arrays stay Arrays.
* **Lists are Arrays, Strings or LazyLists**, exactly as for patterns.
  Prelude functions accept all three and stay lazy when their input is.
  Ranges are Arrays when bounded and lazy when not.
* **Recursion** compiles to the same machinery as `HaskellMatch.fn`: calls
  in tail position use `tail` (constant space), everything else uses the
  segmented stack, and `HaskellMatch.max_depth` applies.
* **Exhaustiveness and redundancy** are enforced for every function,
  including `where`/`let` helpers, `case` expressions and pattern lambdas,
  with the policy you pass as `exhaustive:` (default
  `HaskellMatch.exhaustive`). Errors name the Haskell function and the line
  of the Ruby file (or `.hs` file) it came from.
* **Type scopes.** Each compiled module gets its own type scope: a snapshot
  of the global `HaskellMatch.data` registry when the module is first
  compiled plus its own `data` declarations, so two modules can both declare
  a `Shape`. Ruby code matches on a module's types through the module's
  `fn`, `case_of`, `pattern` and `data` methods, which take the same
  options as `HaskellMatch`'s. Types you want to share between Ruby and
  Haskell are simplest declared globally with `HaskellMatch.data` before
  the module is compiled.
* **Interop.** A name the Haskell does not define (and the Prelude does not
  provide) is called as a method of the host module, so a module can mix
  `def self.helper` with Haskell that calls `helper`. Ruby lambdas, Procs
  and Methods are Haskell functions (`Mod.my_map(->(x) { x * 2 }, [1, 2])`),
  a function returned from Haskell is a Ruby `Proc`, and calling an exported
  function applies like Haskell: `Mod.add3(1, 2, 3)`, `Mod.add3(1).(2).(3)`
  (a partial application) and `Mod.adder(5, 10)` (extra arguments go to the
  returned function) all work. The compiled `Function` objects are available
  as `Mod.haskell_functions` / `Mod.haskell_function(:name)` for `tail`,
  `to_proc`, `curried`, `===` and `decision_tree`.
* **The Prelude** lives in `HaskellMatch::Prelude` and is callable from
  Ruby too (`HaskellMatch::Prelude.take(3, xs)`). It provides the standard
  types `Maybe`, `Either` and `Ordering` (`HaskellMatch::Prelude::Maybe::Just`)
  and the usual functions: `map`, `filter`, `foldr`, `foldl`, `zip`,
  `zipWith`, `take`, `drop`, `takeWhile`, `iterate`, `repeat`, `cycle`,
  `sum`, `product`, `length`, `reverse`, `concat`, `concatMap`, `elem`,
  `lookup`, `words`, `lines`, `show`, `fromIntegral`, `div`, `mod`,
  `compare`, `maybe`, `fromMaybe`, `either`, `error`, `undefined`, the
  `Data.Char` basics and more. `HaskellMatch::Prelude::ARITY.keys` lists
  them all.
* **Debugging.** `HaskellMatch::Haskell.generated_ruby(mod)` returns the
  Ruby a module was compiled to.

## Deep and infinite recursion: expert notes

This section is for readers who intend to recurse hundreds of thousands or
millions of levels deep, or to iterate forever, and want to know exactly what
happens underneath. The short version: haskell_match gives you GHC's
behaviour (recursion limited by memory, loops in constant space) on top of a
VM whose stack is fixed, and the price of that is paid in memory and in the
GC. Every number below was measured on Ruby 3.3.6, x86-64 Linux, with the
default settings; reproduce them with the snippets at the end.

### What a call costs on the stacks

A call `f.(x)` enters the native `call` method (one Ruby control frame for the
C function), matches `x` against the decision tree with no allocation, and
invokes the selected clause body with `rb_proc_call_with_block`. The body is
a Ruby block, so it gets a second control frame. If the body itself calls
`f.(y)`, the whole sequence nests. Each level therefore consumes:

* **Ruby VM stack**: two control frames (the C-function frame and the block
  frame) plus the block's locals and operand stack, roughly 150–300 bytes.
* **Machine (C) stack**: the native method's own frame, the VM re-entry
  (`vm_exec`) that runs the block, and the Rust matcher's scratch space,
  roughly a kilobyte.

Ruby sizes both stacks when a thread or fiber is created and never grows
them: 1 MB VM / 1 MB machine for a thread, 128 KB VM / 512 KB machine for a
fiber (`RubyVM::DEFAULT_PARAMS`). A plain Ruby lambda that recurses with one
frame per level dies at about 11,000 levels on the main thread; a clause body
with two frames per level would die at about 7,000; inside a fiber, at about
480. Nothing at run time can enlarge an existing stack, which is why the
mechanisms below exist.

### Mechanism 1: stack segments (what plain recursion uses)

The native `call` keeps a per-thread nesting counter. When a body is about to
run at a depth that is a multiple of `HaskellMatch.stack_segment` (default
100), the body is run inside a brand-new Fiber instead of on the current
stack. That Fiber has fresh 128 KB / 512 KB stacks; the recursion continues
in it until another 100 levels, when the next segment is started. Segments
form a linked chain of suspended fibers, each waiting for the inner one's
result, which is exactly the shape of a growable stack.

Facts about segments:

* **Default 100 is deliberate.** A single fiber holds about 480 plain levels;
  bodies that call a few helper methods per level use more VM stack, so 100
  leaves a safety factor of four to five. Raising `stack_segment` lowers the
  memory per level (fewer, fuller fibers) but narrows that margin: 400 and
  above overflow a fiber with even the simplest body. Lowering it is always
  safe and only costs memory and fiber creations.
* **Memory per level: about 1.6 KB.** Each segment commits its 128 KB fiber
  VM stack in full (the VM touches both ends of it), so the per-level cost is
  dominated by `128 KB / stack_segment`, not by the frames themselves. 200k
  levels peak at about 300 MB; 1M levels at about 1.6 GB. GHC, by comparison,
  spends about 24 bytes per level of `1 + length xs`. The shape is the same
  as Haskell's (non-tail recursion is linear in depth); the constant is about
  sixty times worse.
* **Memory is reclaimed, lazily.** When the outermost call returns, every
  segment fiber has finished and is unreferenced; the Fiber objects are
  collected at the next GC and their stacks go back to Ruby's fiber pool,
  which marks the pages `MADV_FREE`. The kernel reclaims those pages under
  memory pressure, so RSS stays high after a deep call even though the
  memory is available (`LazyFree` in `/proc/self/smaps_rollup` shows it:
  288 MB of a 325 MB RSS after a 200k-deep call). Repeated deep calls reuse
  the pooled stacks and do not grow RSS further. This is not a leak; it is
  the pool keeping what it once needed.
* **First call is slow, later calls are fast.** Committing fresh fiber stacks
  page-faults every page: the first 400k-deep call took 3.2 s, the second
  0.6 s, the third 0.35 s (0.9 µs per level); at 1M depth, 8.9 s then 2.8 s.
  Budget for the cold run if a deep recursion happens once.
* **GC cost grows with live depth.** Fiber objects are not write-barrier
  protected (their stacks change on every instruction, so no barrier could
  track them), and CRuby therefore re-marks the VM and machine stacks of
  every suspended segment on every collection, minor ones included. A GC
  during a pending recursion costs O(live stack bytes): at 200k levels a
  minor GC takes about 165 ms instead of 2 ms. Nothing outside CRuby can
  change *that* a suspended fiber is rescanned; what can be changed is how
  much there is to scan, and that is what deep mode below does (28 ms for the
  same collection). `defer` avoids the fiber stacks altogether, and
  `GC.disable` around a known deep computation, or a larger
  `RUBY_GC_HEAP_INIT_SLOTS`, reduces the number of collections.
* **Semantics across a segment boundary.** Exceptions propagate normally
  (`rb_fiber_resume` re-raises them in the parent), but the backtrace only
  covers the innermost segment. Non-local exits do not cross: `throw` to a
  `catch` outside the segment raises `UncaughtThrowError`, and `break` or
  `return` out of a body proc raises `LocalJumpError` at the boundary.
  Within 100 levels of the `catch`, everything behaves as in one stack.
  `Fiber.yield` inside a body yields the segment fiber, not yours.
* **Threads and Ractors.** The depth counter is per OS thread, so each Ruby
  Thread and each Ractor recurses independently. If your own Fiber runs a
  deep recursion, the counter is shared with the thread that created it; the
  only effect is that segments may start a little earlier than needed.

### Deep mode: the same recursion with no native frames on the stack

Where do the 1.6 KB per level come from, given that an idle fiber commits only
about 13 KB? From the C frames. In the default (native) mode the body is
invoked by the Rust matcher: the machine stack holds, per level, the C
function frame of `call`, the VM re-entry that runs the block, and the
matcher's scratch space, about 1.3 KB, all of which the GC also scans
conservatively because it cannot know which words are references. The Ruby
frames themselves are small.

`deep: true` moves the body invocation into Ruby:

```ruby
count = HaskellMatch.fn(:count, deep: true) do
  on([])        { 0 }
  on([_, *xs])  { |xs| 1 + count.(xs) }
end
```

`call` becomes a generated Ruby method of exact arity that asks the native
`prepare` for the selected body and its bound values, then calls the body
itself. A Ruby-to-Ruby call pushes no C frame, so while the recursion is
pending the machine stack holds nothing of ours; segments, `tail`, `defer`
and `max_depth` work exactly as before (the trampoline is reimplemented in
Ruby, with the same chunked continuation stack). Measured against native
mode at 200k levels:

| mode   | shallow call | memory per level | minor GC at 200k depth | 200k levels, warm |
|--------|-------------:|-----------------:|-----------------------:|------------------:|
| native |     ~230 ns  |        ~1.6 KB   |              ~165 ms   |          0.94 s   |
| deep   |     ~460 ns  |        ~105 B    |               ~28 ms   |          0.29 s   |

A million plain levels in deep mode take about 3 s and 300 MB. The trade is
clear-cut: deep mode costs an extra Ruby frame on *every* call, which doubles
the time of a shallow call, and in exchange brings a level within a factor of
four of GHC's 24 bytes and cuts GC marking six-fold. Native mode remains the
default because most functions never recurse; set `deep: true` on the ones
that do, or `HaskellMatch.deep_by_default = true` for a codebase that is
recursive throughout. A deep-mode function may call native-mode functions
and vice versa, including through `tail`.

### Mechanism 2: tail calls (constant space)

```ruby
go = HaskellMatch.fn(:go) do
  on(acc, [])        { |acc| acc }
  on(acc, [x, *xs])  { |acc, x, xs| go.tail(acc + x, xs) }
end
```

`go.tail(args...)` returns a `HaskellMatch::TailCall` marker. The native
`call` sees it, replaces the current arguments with the marker's and matches
again on the same frame; nothing is pushed on any stack and nothing counts
towards `max_depth`. The target may be a different function, so mutual
recursion (`even`/`odd`) loops in constant space too. Cost: one small `Data`
allocation per iteration, about 1.1 µs per level including the list slice
(a shared, copy-on-write subarray). This is the right tool for anything that
is a loop in Haskell: accumulators, folds, state machines, servers. A marker
returned anywhere but as the body's final value is just a value (`go.tail(1)`
outside a call is an ordinary object).

### Mechanism 3: deferred calls (cheap depth)

```ruby
length = HaskellMatch.fn(:length) do
  on([])        { 0 }
  on([_, *xs])  { |xs| length.defer(xs) { |n| 1 + n } }    # 1 + length xs
end
```

`f.defer(args...) { |result| ... }` is a tail call that carries a
continuation. The native `call` pushes the block on a stack it owns and
continues with the call; when a body finally returns an ordinary value, the
pending blocks are applied to it last-in first-out, each possibly returning
another marker. The recursion's stack lives on the heap, so it is bounded by
memory alone and `max_depth` never triggers.

That stack is deliberately a chain of 256-element Ruby arrays rather than one
growing array. Ruby's GC is generational: an old array that is written to is
put on the remembered set and rescanned in full by every minor collection, so
a single million-element stack would have made each GC O(depth). A full chunk
is never written again, gets promoted, and is skipped by minor GCs; only the
current chunk is rescanned. The difference is large: 2M levels took 31 s with
one array and 8.9 s with chunks (1M: 5.4 s vs 3.8 s), and peak memory fell
from 770 MB to 470 MB because the old array's doubling growth is gone.

Per level `defer` costs one Proc plus its environment (about 200 bytes) and
about 4 µs, mostly the allocation and the minor GCs it triggers (878 minor
GCs over a 2M-deep run, each cheap). Prefer `defer` over plain recursion when
depth is known to be large and memory matters; prefer plain recursion when it
is not, since `defer` requires writing the continuation by hand and runs the
continuation outside the body's frame (`self` and closure variables are those
of the block, as usual).

### The depth guard

`HaskellMatch.max_depth` (default 250,000) bounds the nesting counter from
mechanism 1. Past it, the next nested call raises
`HaskellMatch::StackOverflowError` instead of allocating another segment. At
1.6 KB per level the default corresponds to about 400 MB, the point of the
limit being that a runaway recursion fails loudly rather than exhausting the
machine, which is what GHC's stack limit (80% of RAM by default) is for too.
Set it higher or to 0 (unlimited) for a computation that legitimately needs
more; `tail` and `defer` never count towards it. The counter is restored
exactly even when a body leaves by exception, so a caught error deep in a
recursion does not shift later limits.

### Choosing

| Pattern of recursion                      | Use                      | Space       | Time per level (warm) |
|-------------------------------------------|--------------------------|-------------|-----------------------|
| Loop with accumulator, fold, state machine| `f.tail(...)`            | O(1)        | ~1.1 µs               |
| Deep non-tail recursion, depth known large| `f.defer(...) { }`       | ~200 B/level| ~4 µs                 |
| Ordinary recursion, depth moderate        | plain `f.(...)`          | ~1.6 KB/level (≤ `max_depth`) | ~0.9 µs (first call slower) |
| Ordinary recursion, depth large           | `deep: true` + plain `f.(...)` | ~105 B/level (≤ `max_depth`) | ~1.5 µs, half the GC time |
| Infinite data                             | `HaskellMatch.lazy` + patterns | per element forced | per element |

Infinite recursion in the Haskell sense, a producer that never returns, is
expressed as a lazy list consumed by `tail`-recursive or bounded consumers:
`take.(n, HaskellMatch::LazyList.iterate(1) { |x| x * 2 })` forces exactly
`n` cells and no more, and each forced cell is memoised, so sharing works as
in Haskell (two consumers of the same list see the same elements, computed
once). An Enumerator passed directly is re-wrapped on each call, iterating
from its start, which keeps calls referentially transparent at the cost of
recomputing from scratch per call; keep a `LazyList` in a variable when the
elements are expensive.

### Reproducing the measurements

```ruby
require "haskell_match"
HaskellMatch.max_depth = 0
count = HaskellMatch.fn(:count) { on("[]") { 0 }; on("(_:xs)") { |xs| 1 + count.(xs) } }
dlen  = HaskellMatch.fn(:dlen)  { on("[]") { 0 }; on("(_:xs)") { |xs| dlen.defer(xs) { |n| 1 + n } } }
go    = HaskellMatch.fn(:go)    { on("acc", "[]") { |acc| acc }; on("acc", "(x:xs)") { |acc, x, xs| go.tail(acc + x, xs) } }

rss = -> { File.read("/proc/self/status")[/VmRSS:\s+(\d+)/, 1].to_i / 1024 }
list = (1..200_000).to_a
before = rss.(); count.(list); puts "segments: +#{rss.() - before} MB"   # ~300 MB, ~1.6 KB/level
before = rss.(); dlen.(list);  puts "defer:    +#{rss.() - before} MB"   # ~45 MB
before = rss.(); go.(0, list); puts "tail:     +#{rss.() - before} MB"   # ~0 MB
GC.start; puts File.read("/proc/self/smaps_rollup")[/LazyFree:.*/]     # reclaimable pages
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
