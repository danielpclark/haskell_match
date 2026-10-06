# frozen_string_literal: true

require_relative "test_helper"

# Patterns written in place, without quotes.
class PatternAstTest < Minitest::Test
  include TestHelpers

  def render(&blk)
    HaskellMatch.pattern(&blk).source
  end

  def test_rendering
    assert_equal "_", render { _ }
    assert_equal "x", render { x }
    assert_equal "x", render { var(:x) }
    assert_equal "Nothing", render { Nothing }
    assert_equal "Just x", render { Just(x) }
    assert_equal "Just x", render { Just[x] }
    assert_equal "Just (Just _)", render { Just(Just(_)) }
    assert_equal "Just (Just _)", render { Just[Just[_]] }
    assert_equal "Node Leaf v (Node _ _ _)", render { Node(Leaf, v, Node(_, _, _)) }
    assert_equal "[]", render { [] }
    assert_equal "[a, b]", render { [a, b] }
    assert_equal "(x:xs)", render { [x, *xs] }
    assert_equal "(x:y:_)", render { [x, y, *_] }
    assert_equal "(x:xs)", render { cons(x, xs) }
    assert_equal "(a, b)", render { tuple(a, b) }
    assert_equal "()", render { unit }
    assert_equal "all@(x:_)", render { as(all, [x, *_]) }
    assert_equal "all@(x:_)", render { as(:all, cons(x, _)) }
    assert_equal "~(a, b)", render { lazy(tuple(a, b)) }
    assert_equal "!x", render { bang(x) }
    assert_equal "Person {name = n, age = _}", render { Person(name: n, age: _) }
    assert_equal "Person {name = n, ..}", render { Person(name: n, **_) }
    assert_equal "Person \"Ann\" 30", render { Person["Ann", 30] }
    assert_equal "0", render { 0 }
    assert_equal "Just (-1)", render { Just(-1) }
    assert_equal "-1", render { -1 }
    assert_equal "1.5", render { 1.5 }
    assert_equal "Just (-2.5)", render { Just(-2.5) }
    assert_equal "\"hi\\n\"", render { "hi\n" }
    assert_equal "\"hi\"", render { str("hi") }
    # a String handed straight to `on` is quoted pattern syntax
    assert_equal 1, fn { on("x") { |x| x } }.(1)
    assert_equal :s, fn { on(str("s")) { :s }; on(_) { :other } }.("s")
    assert_equal "\"q\\\"q\"", render { 'q"q' }
    assert_equal "'c'", render { char("c") }
    assert_equal ":ok", render { :ok }
    assert_equal ":\"with space\"", render { :"with space" }
    assert_equal "True", render { true }
    assert_equal "(True, False)", render { tuple(true, false) }
    assert_equal "Just [1, 2]", render { Just([1, 2]) }
    assert_equal "Just (x:xs)", render { Just([x, *xs]) }
    assert_equal "Just \"s\"", render { Just("s") }
  end

  def test_in_place_patterns_match_like_quoted_ones
    length = fn(:length) do
      on([]) { 0 }
      on([_, *xs]) { |xs| 1 + length.(xs) }
    end
    assert_equal 3, length.([1, 2, 3])
    assert_equal 3, length.("abc")

    describe = fn(:describe) do
      on(Just(Just(_))) { :nested }
      on(Just(x), guard: ->(x) { x > 10 }) { :big }
      on(Just(x)) { |x| x }
      on(Nothing) { :none }
    end
    assert_equal :nested, describe.(Just[Just[1]])
    assert_equal :big, describe.(Just[11])
    assert_equal 1, describe.(Just[1])
    assert_equal :none, describe.(Nothing)

    greet = fn(:greet) do
      on(Person(name: "Ann")) { :ann }
      on(Person(name: n, age: a), guard: ->(a) { a >= 18 }) { |n| "#{n} (adult)" }
      on(Person(name: n, **_)) { |n, age| "#{n} (#{age})" }
    end
    assert_equal :ann, greet.(Person.new("Ann", 1))
    assert_equal "Bob (adult)", greet.(Person.new("Bob", 30))
    assert_equal "Cid (5)", greet.(Person.new("Cid", 5))

    zip = fn(:zip) do
      on([x, *xs], [y, *ys]) { |x, xs, y, ys| [[x, y]] + zip.(xs, ys) }
      on(_, _) { [] }
    end
    assert_equal [[1, :a]], zip.([1, 2], [:a])

    mixed = fn(:mixed) { on("Just x", tuple(a, b)) { |x, a, b| x + a + b }; on("Nothing", _) { 0 } }
    assert_equal 6, mixed.(Just[1], [2, 3])

    fib = fn(:fib) { on(0) { 0 }; on(1) { 1 }; on(n) { |n| fib.(n - 1) + fib.(n - 2) } }
    assert_equal 55, fib.(10)

    initial = fn(:initial) { on([char("a"), *_]) { :a }; on(str("")) { :empty }; on(_) { :other } }
    assert_equal :a, initial.("apple")
    assert_equal :empty, initial.("")
    assert_equal :other, initial.("pear")
  end

  def test_exhaustiveness_and_errors_apply_to_in_place_patterns
    err = assert_raises(HaskellMatch::NonExhaustiveError) { fn { on(Just(x)) { |x| x } } }
    assert_equal ["Nothing"], err.missing
    err = assert_raises(HaskellMatch::RedundantClauseError) { fn(:r) { on(_) { 1 }; on(Just(x)) { 2 } } }
    assert_includes err.message, "r Just x = ..."
    assert_raises(HaskellMatch::ArityError) { fn { on(Just) { 1 } } }
    assert_raises(HaskellMatch::ArityError) { fn { on(Just(a, b)) { 1 } } }
    assert_raises(HaskellMatch::UnknownConstructorError) { fn { on(Nope(x)) { 1 } } }
    assert_raises(HaskellMatch::PatternTypeError) { fn { on(Just(x)) { 1 }; on([x, *xs]) { 2 } } }
    assert_raises(HaskellMatch::DuplicateVariableError) { fn { on(tuple(x, x)) { 1 } } }
    assert_raises(HaskellMatch::FieldError) { fn { on(Person(nome: n)) { 1 } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on(nil) { 1 } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on({ a: 1 }) { 1 } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on(1..2) { 1 } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on([*xs, x]) { 1 } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on(Person(n, name: n)) { 1 } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on(char("ab")) { 1 } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on(var(:Upper)) { 1 } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on(Float::INFINITY) { 1 } } }
  end

  def helper_value
    42
  end

  def test_bare_names_defer_to_methods_of_the_enclosing_object
    # `helper_value` is a method of this test, so it is a call, not a variable
    f = fn { on(x) { |x| x + helper_value } }
    assert_equal 43, f.(1)
    # a variable with that name is still available explicitly
    g = fn { on(Just(var(:helper_value))) { |helper_value| helper_value }; on(Nothing) { 0 } }
    assert_equal 7, g.(Just[7])
  end

  def test_pattern_objects_case_of_and_hdef
    p = HaskellMatch.pattern { Just([x, *rest]) }
    assert_equal "Just (x:rest)", p.source
    assert_equal({ x: 1, rest: [2] }, p.match(Just[[1, 2]]))
    assert_raises(ArgumentError) { HaskellMatch.pattern("x") { x } }
    r = HaskellMatch.case_of(Just[3]) { on(Just(x)) { |x| x * 2 }; on(Nothing) { 0 } }
    assert_equal 6, r
    assert_equal "#<HaskellMatch pattern Just x>",
                 HaskellMatch::PatternAST::ConApp.new("Just", [HaskellMatch::PatternAST::Var.new(:x)]).inspect
    assert_equal 6, Shapes.new.area(Rect.new(2, 3))
  end

  class Shapes
    extend HaskellMatch::DSL
    include Shape

    hdef :area do
      on(Circle(r)) { |r| 3 * r * r }
      on(Rect(w, h)) { |w, h| w * h }
      on(Tri(a, b, c)) { |a, b, c| a + b + c }
    end
  end
end
