# frozen_string_literal: true

require_relative "test_helper"

class FunctionTest < Minitest::Test
  include TestHelpers

  def test_constructor_matching_and_bindings
    f = fn(:from_maybe) do
      on("d", "Nothing") { |d| d }
      on("_", "Just x") { |x| x }
    end
    assert_equal 5, f.(0, Just.new(5))
    assert_equal 0, f.(0, Nothing)
    assert_equal 2, f.arity
    assert_equal "from_maybe", f.name
    assert_equal [["d"], ["x"]], f.bindings
    assert_equal "#<HaskellMatch::Function from_maybe/2>", f.inspect
  end

  def test_lists
    len = fn(:len) { on("[]") { 0 }; on("(_:xs)") { |xs| 1 + len.(xs) } }
    assert_equal 0, len.([])
    assert_equal 4, len.([1, 2, 3, 4])

    sum = fn(:sum) { on("[]") { 0 }; on("(x:xs)") { |x, xs| x + sum.(xs) } }
    assert_equal 10, sum.((1..4).to_a)

    second = fn(:second) do
      on("[]") { nil }
      on("[_]") { nil }
      on("(_:y:_)") { |y| y }
    end
    assert_nil second.([])
    assert_nil second.([1])
    assert_equal 2, second.([1, 2])
    assert_equal 2, second.([1, 2, 3])

    exact = fn(:exact) do
      on("[a, b]") { |a, b| [:two, a, b] }
      on("[a]") { |a| [:one, a] }
      on("_") { :other }
    end
    assert_equal [:two, 1, 2], exact.([1, 2])
    assert_equal [:one, 1], exact.([1])
    assert_equal :other, exact.([1, 2, 3])
    assert_equal :other, exact.([])
  end

  def test_list_tails_are_real_arrays_sharing_nothing_observable
    tail = fn(:tail) { on("(_:xs)") { |xs| xs }; on("[]") { [] } }
    src = [1, 2, 3]
    t = tail.(src)
    assert_equal [2, 3], t
    t << 4
    assert_equal [1, 2, 3], src
    assert_equal [2, 3, 4], t
    assert_equal [], tail.([1])
    assert_equal [], tail.(tail.(tail.([1, 2, 3])))
  end

  def test_zip_multi_argument
    zip = fn(:zip) do
      on("(x:xs)", "(y:ys)") { |x, xs, y, ys| [[x, y]] + zip.(xs, ys) }
      on("_", "_") { [] }
    end
    assert_equal [[1, :a], [2, :b]], zip.([1, 2, 3], %i[a b])
    assert_equal [], zip.([], [1])
  end

  def test_tuples_and_unit
    swap = fn(:swap) { on("(a, b)") { |a, b| [b, a] } }
    assert_equal [2, 1], swap.([1, 2])
    err = assert_raises(HaskellMatch::TypeMismatchError) { swap.([1, 2, 3]) }
    assert_includes err.message, "expected a value of type (,)"
    assert_raises(HaskellMatch::TypeMismatchError) { swap.(5) }

    unit = fn(:unit) { on("()") { :unit } }
    assert_equal :unit, unit.([])
    assert_raises(HaskellMatch::TypeMismatchError) { unit.([1]) }

    triple = fn(:triple) { on("(a, (b, c), _)") { |a, b, c| a + b + c } }
    assert_equal 6, triple.([1, [2, 3], :ignored])
  end

  def test_booleans
    f = fn(:not) { on("True") { false }; on("False") { true } }
    assert_equal false, f.(true)
    assert_equal true, f.(false)
    assert_raises(HaskellMatch::TypeMismatchError) { f.(nil) }
    assert_raises(HaskellMatch::TypeMismatchError) { f.(1) }
    both = fn(:and) do
      on("True", "True") { true }
      on("_", "_") { false }
    end
    assert both.(true, true)
    refute both.(true, false)
  end

  def test_integer_literals
    fib = fn(:fib) do
      on("0") { 0 }
      on("1") { 1 }
      on("n") { |n| fib.(n - 1) + fib.(n - 2) }
    end
    assert_equal 55, fib.(10)
    assert_equal 0, fib.(0.0) # 0 == 0.0 as in Haskell's overloaded literals
    neg = fn(:neg) { on("-1") { :minus_one }; on("(-2)") { :minus_two }; on("_") { :other } }
    assert_equal :minus_one, neg.(-1)
    assert_equal :minus_two, neg.(-2)
    assert_equal :other, neg.(1)
    hex = fn(:hex) { on("0xFF") { :ff }; on("_") { nil } }
    assert_equal :ff, hex.(255)
  end

  def test_big_and_float_literals
    big = fn(:big) do
      on("123456789012345678901234567890") { :big }
      on("4611686018427387904") { :just_past_fixnum }
      on("_") { :other }
    end
    assert_equal :big, big.(123_456_789_012_345_678_901_234_567_890)
    assert_equal :just_past_fixnum, big.(2**62)
    assert_equal :other, big.(2**62 + 1)
    assert_equal :other, big.(1)
    fl = fn(:fl) { on("1.5") { :one_and_half }; on("2.0") { :two }; on("_") { :other } }
    assert_equal :one_and_half, fl.(1.5)
    assert_equal :two, fl.(2)
    assert_equal :two, fl.(2.0)
    assert_equal :two, fl.(Rational(2, 1))
    assert_equal :other, fl.(2.1)
    assert_equal :other, fl.("2") # `_` accepts anything
  end

  def test_strings_are_lists_of_characters
    # String = [Char]: list patterns destructure Ruby Strings
    f = fn(:f) do
      on("\"\"") { :empty }
      on("\"hello\"") { :hi }
      on("\"\\n\"") { :newline }
      on("['y', _]") { :y_then_one }
      on("(c:cs)") { |c, cs| [c, cs] }
    end
    assert_equal :empty, f.("")
    assert_equal :hi, f.("hello")
    assert_equal :hi, f.("hello".b)
    assert_equal :newline, f.("\n")
    assert_equal :y_then_one, f.("yx")
    assert_equal ["h", "ey"], f.("hey")
    assert_equal ["h", ""], f.("h")
    assert_equal ["é", "té"], f.("été")
    # the same patterns work on arrays of one-character strings
    assert_equal :hi, f.(%w[h e l l o])
    assert_equal :empty, f.([])
    assert_equal [1, [2]], f.([1, 2])
    # a tail bound from a string is a String, independent of the original
    tail = f.("abc")[1]
    assert_equal "bc", tail
    tail << "!"
    assert_equal "bc!", tail
  end

  def test_char_literals
    g = fn(:g) { on("'x'") { :x }; on("'é'") { :e_acute }; on("c") { |c| c } }
    assert_equal :x, g.("x")
    assert_equal :e_acute, g.("é")
    assert_equal "xy", g.("xy")
    # Char and String are different types, as in Haskell
    assert_raises(HaskellMatch::PatternTypeError) { fn { on("'x'") { 1 }; on("\"x\"") { 2 } } }
    initial = fn(:initial) { on("('a':_)") { :a }; on("('b':_)") { :b }; on("_") { :other } }
    assert_equal :a, initial.("apple")
    assert_equal :b, initial.("banana")
    assert_equal :other, initial.("cherry")
    assert_equal :other, initial.("")
  end

  def test_symbol_literals
    sym = fn(:sym) { on(":ok") { 1 }; on(":error") { 2 }; on(":\"with space\"") { 3 }; on("_") { 0 } }
    assert_equal 1, sym.(:ok)
    assert_equal 2, sym.(:error)
    assert_equal 3, sym.(:"with space")
    assert_equal 0, sym.(:other)
    assert_equal 1, sym.("ok".to_sym)
    # `_` matches anything, so a String simply takes the wildcard clause
    assert_equal 0, sym.("ok")
    only = fn(:only, exhaustive: false) { on(":ok") { 1 } }
    assert_raises(HaskellMatch::TypeMismatchError) { only.("ok") }
  end

  def test_wildcards_match_anything
    # Haskell's `_` and variables never fail to match.
    f = fn { on("Just x") { |x| x }; on("_") { :other } }
    assert_equal :other, f.(Nothing)
    assert_equal :other, f.(5)
    assert_equal :other, f.(nil)
    assert_equal :other, f.([1])
    g = fn { on("0") { :zero }; on("n") { |n| n } }
    assert_equal "0", g.("0")
    assert_nil g.(nil)
    assert_equal Rational(1, 3), g.(Rational(1, 3))
    h = fn { on("True") { 1 }; on("_") { 2 } }
    assert_equal 2, h.(nil)
    # (a clause after `[]` and `(x:_)` would be redundant, as in Haskell)
    k = fn { on("[]") { :nil }; on("(x:_)") { |x| x } }
    assert_equal :nil, k.("")
    assert_equal 1, k.([1])
    assert_raises(HaskellMatch::TypeMismatchError) { k.({ a: 1 }) }
  end

  def test_type_mismatch_when_no_pattern_can_accept_the_value
    # Haskell rejects these calls at compile time; here the error is raised
    # when a value matches none of the patterns and no wildcard remains.
    f = fn { on("Just x") { |x| x }; on("Nothing") { 0 } }
    err = assert_raises(HaskellMatch::TypeMismatchError) { f.(5) }
    assert_includes err.message, "expected a value of type Maybe but got 5 (Integer)"
    assert_raises(HaskellMatch::TypeMismatchError) { f.(nil) }
    assert_raises(HaskellMatch::TypeMismatchError) { f.(Just) }
    assert_raises(HaskellMatch::TypeMismatchError) { f.(Left.new(1)) }
    b = fn { on("True") { 1 }; on("False") { 0 } }
    err = assert_raises(HaskellMatch::TypeMismatchError) { b.(nil) }
    assert_includes err.message, "expected a value of type Bool but got nil (NilClass)"
    n = fn(exhaustive: false) { on("0") { :zero }; on("1") { :one } }
    err = assert_raises(HaskellMatch::TypeMismatchError) { n.("0") }
    assert_includes err.message, "expected a value of type Num"
    assert_raises(HaskellMatch::MatchError) { n.(2) }
    l = fn(exhaustive: false) { on("(x:_)") { |x| x } }
    assert_raises(HaskellMatch::TypeMismatchError) { l.(5) }
    assert_raises(HaskellMatch::MatchError) { l.([]) }
    # a wrong-typed value nested inside a well-typed one
    d = fn { on("Just (Just x)") { |x| x }; on("Just Nothing") { 0 }; on("Nothing") { -1 } }
    assert_raises(HaskellMatch::TypeMismatchError) { d.(Just.new(5)) }
  end

  def test_nested_patterns
    f = fn(:deep) do
      on("Just (Just (x:_))") { |x| [:deep, x] }
      on("Just (Just [])") { :empty }
      on("Just Nothing") { :inner_nothing }
      on("Nothing") { :nothing }
    end
    assert_equal [:deep, 7], f.(Just.new(Just.new([7, 8])))
    assert_equal :empty, f.(Just.new(Just.new([])))
    assert_equal :inner_nothing, f.(Just.new(Nothing))
    assert_equal :nothing, f.(Nothing)
    assert_raises(HaskellMatch::TypeMismatchError) { f.(Just.new(5)) }
  end

  def test_as_patterns
    f = fn(:dup_head) do
      on("all@(x:_)") { |all, x| [x] + all }
      on("[]") { [] }
    end
    assert_equal [1, 1, 2], f.([1, 2])
    g = fn { on("m@(Just n@(Just _))") { |m, n| [m, n] }; on("_") { nil } }
    v = Just.new(Just.new(1))
    assert_equal [v, Just.new(1)], g.(v)
  end

  def test_lazy_patterns
    f = fn(:lazy) { on("~(Just x)") { |x| x } }
    assert_equal 1, f.(Just.new(1))
    err = assert_raises(HaskellMatch::IrrefutablePatternError) { f.(Nothing) }
    assert_includes err.message, "irrefutable pattern failed"
    g = fn { on("(a, ~(b, c))") { |a, b, c| [a, b, c] } }
    assert_equal [1, 2, 3], g.([1, [2, 3]])
    assert_raises(HaskellMatch::IrrefutablePatternError) { g.([1, [2]]) }
    h = fn { on("~[x, 1]") { |x| x } }
    assert_equal :a, h.([:a, 1])
    assert_raises(HaskellMatch::IrrefutablePatternError) { h.([:a, 2]) }
    # a lazy pattern never influences clause selection
    k = fn { on("~(Just x)", "True") { |x| [:first, x] }; on("_", "False") { :second } }
    assert_equal :second, k.(Nothing, false)
    assert_raises(HaskellMatch::IrrefutablePatternError) { k.(Nothing, true) }
  end

  def test_bang_patterns_are_accepted
    f = fn { on("!x") { |x| x } }
    assert_equal 3, f.(3)
    g = fn { on("Just !x") { |x| x }; on("Nothing") { 0 } }
    assert_equal 3, g.(Just.new(3))
  end

  def test_record_patterns
    f = fn(:greeting) do
      on("Person { name = \"Ann\" }") { :ann }
      on("Person { age = a }", guard: ->(a) { a >= 18 }) { |a| [:adult, a] }
      on("Person { name, .. }") { |name, age| [:minor, name, age] }
    end
    assert_equal :ann, f.(Person.new("Ann", 1))
    assert_equal [:adult, 30], f.(Person.new("Bob", 30))
    assert_equal [:minor, "Cid", 5], f.(Person.new("Cid", 5))
    g = fn { on("Person {}") { :person } }
    assert_equal :person, g.(Person.new("x", 1))
    h = fn { on("Person n a") { |n, a| [n, a] } }
    assert_equal ["x", 1], h.(Person.new("x", 1))
  end

  def test_guards_and_otherwise
    sign = fn(:sign) do
      on("n", guard: ->(n) { n > 0 }) { 1 }
      on("n", where: ->(n) { n < 0 }) { -1 }
      on("_", guard: otherwise) { 0 }
    end
    assert_equal [1, -1, 0], [5, -5, 0].map { |x| sign.(x) }
    classify = fn(:classify) do
      on("Just x", guard: ->(x) { x.even? }) { :even }
      on("Just x", guard: ->(x) { x > 100 }) { :big_odd }
      on("Just _") { :odd }
      on("Nothing") { :none }
    end
    assert_equal :even, classify.(Just.new(2))
    assert_equal :big_odd, classify.(Just.new(101))
    assert_equal :odd, classify.(Just.new(3))
    assert_equal :none, classify.(Nothing)
  end

  def test_guard_binding_styles
    f = fn do
      on("(a, b)", guard: ->(b:) { b > a_threshold }) { :big }
      on("(a, b)", guard: ->(b, a) { b == a }) { :same }
      on("(a, b)", guard: ->(*all) { all.sum.zero? }) { :zero_sum }
      on("_") { :other }
    end
    assert_equal :big, f.([1, 100])
    assert_equal :same, f.([2, 2])
    assert_equal :zero_sum, f.([1, -1])
    assert_equal :other, f.([1, 2])
  end

  def a_threshold
    50
  end

  def test_body_binding_styles
    f = fn { on("(x:xs)") { |xs| xs }; on("[]") { [] } }
    assert_equal [2, 3], f.([1, 2, 3])
    g = fn { on("(x:xs)") { |xs, x| [xs, x] }; on("[]") { [] } }
    assert_equal [[2], 1], g.([1, 2])
    h = fn { on("(x:xs)") { |x:, xs:| [x, xs] }; on("[]") { [] } }
    assert_equal [1, [2]], h.([1, 2])
    k = fn { on("(x:xs)") { |**all| all }; on("[]") { {} } }
    assert_equal({ x: 1, xs: [2] }, k.([1, 2]))
    s = fn { on("(x:xs)") { |*all| all }; on("[]") { [] } }
    assert_equal [1, [2]], s.([1, 2])
    none = fn { on("(x:xs)") { :ignored }; on("[]") { :empty } }
    assert_equal :ignored, none.([1])
    l = fn { on("(a, b)", &->(b, a) { [a, b] }) }
    assert_equal [1, 2], l.([1, 2])
    partial_kw = fn { on("(a, b, c)") { |c:| c } }
    assert_equal 3, partial_kw.([1, 2, 3])
    destructuring = fn { on("(a, b)") { |(p, q), b| [p, q, b] } }
    assert_equal [1, 2, 3], destructuring.([[1, 2], 3])
  end

  def test_body_parameter_errors
    err = assert_raises(HaskellMatch::DefinitionError) { fn(:bad) { on("(x:xs)") { |ys| ys }; on("[]") { [] } } }
    assert_includes err.message, "body of clause 1 of 'bad' names :ys but the pattern binds x, xs"
    err = assert_raises(HaskellMatch::DefinitionError) { fn(:bad) { on("[]") { |x| x }; on("_") { 1 } } }
    assert_includes err.message, "binds no variables"
    assert_raises(HaskellMatch::DefinitionError) { fn { on("(a, b)") { |a, c:| } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on("(a, b)") { |zz:| } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on("(a, b)", guard: ->(q) { q }) { 1 }; on("_") { 2 } } }
  end

  def test_definition_errors
    assert_raises(ArgumentError) { HaskellMatch.fn(:x) }
    assert_raises(HaskellMatch::DefinitionError) { fn { } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on { 1 } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on("x") } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on(Object.new) { 1 } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on("x", guard: 5) { 1 } } }
    err = assert_raises(HaskellMatch::ClauseArityError) { fn(:f) { on("x", "y") { 1 }; on("x") { 2 } } }
    assert_includes err.message, "different numbers of arguments"
    assert_raises(ArgumentError) { fn(exhaustive: :maybe) { on("x") { 1 } } }
  end

  def test_compile_error_classes
    assert_raises(HaskellMatch::PatternSyntaxError) { fn { on("(x") { 1 } } }
    assert_raises(HaskellMatch::UnknownConstructorError) { fn { on("Nope x") { 1 } } }
    assert_raises(HaskellMatch::ArityError) { fn { on("Just") { 1 } } }
    assert_raises(HaskellMatch::ArityError) { fn { on("Just a b") { 1 } } }
    assert_raises(HaskellMatch::DuplicateVariableError) { fn { on("(x, x)") { 1 } } }
    assert_raises(HaskellMatch::DuplicateVariableError) { fn { on("x", "x") { 1 } } }
    assert_raises(HaskellMatch::FieldError) { fn { on("Person { nome = n }") { 1 } } }
    assert_raises(HaskellMatch::FieldError) { fn { on("Just { value = v }") { 1 } } }
    err = assert_raises(HaskellMatch::PatternTypeError) { fn { on("Just x") { 1 }; on("(x:xs)") { 2 } } }
    assert_includes err.message, "couldn't match type 'Maybe' (clause 1) with '[]' (clause 2)"
    assert_raises(HaskellMatch::PatternTypeError) { fn { on("0") { 1 }; on("\"s\"") { 2 } } }
    assert_raises(HaskellMatch::PatternTypeError) { fn { on("(a, b)") { 1 }; on("(a, b, c)") { 2 } } }
    assert_raises(HaskellMatch::PatternTypeError) { fn { on("True") { 1 }; on("Nothing") { 2 } } }
    assert_raises(HaskellMatch::PatternTypeError) { fn { on("Just 0") { 1 }; on("Just :s") { 2 }; on("_") { 3 } } }
    # every compile error is a CompileError and an Error
    assert HaskellMatch::PatternTypeError < HaskellMatch::CompileError
    assert HaskellMatch::CompileError < HaskellMatch::Error
    assert HaskellMatch::MatchError < HaskellMatch::Error
    # messages mention the clause
    err = assert_raises(HaskellMatch::UnknownConstructorError) { fn { on("Nothing") { 1 }; on("Jsut x") { 2 } } }
    assert_includes err.message, "clause 2: not in scope: data constructor 'Jsut'"
  end

  def test_partial_functions_raise_match_error_at_runtime
    head = fn(:head, exhaustive: false) { on("(x:_)") { |x| x } }
    assert_equal 1, head.([1])
    err = assert_raises(HaskellMatch::MatchError) { head.([]) }
    assert_equal "Non-exhaustive patterns in head", err.message
    refute_kind_of HaskellMatch::TypeMismatchError, err
    guarded = fn(:pos, exhaustive: false) { on("n", guard: ->(n) { n > 0 }) { :pos } }
    assert_raises(HaskellMatch::MatchError) { guarded.(-1) }
  end

  def test_argument_count_errors
    f = fn { on("a", "b") { |a, b| a + b } }
    err = assert_raises(ArgumentError) { f.(1) }
    assert_includes err.message, "wrong number of arguments (given 1, expected 2)"
    assert_raises(ArgumentError) { f.(1, 2, 3) }
    assert_raises(ArgumentError) { f.() }
  end

  def test_definition_block_sees_methods_of_the_enclosing_object
    f = fn { on("x") { |x| x + a_threshold } }
    assert_equal 51, f.(1)
    g = fn { |m| m.on("x") { |x| x + a_threshold } }
    assert_equal 52, g.(2)
  end

  def test_exceptions_from_bodies_and_guards_propagate
    boom = Class.new(StandardError)
    f = fn { on("Just x") { |x| raise boom, "body #{x}" }; on("Nothing") { 0 } }
    err = assert_raises(boom) { f.(Just.new(1)) }
    assert_equal "body 1", err.message
    assert(err.backtrace.any? { |l| l.include?("function_test.rb") })
    g = fn { on("x", guard: ->(x) { raise boom, "guard #{x}" }) { 1 }; on("_") { 2 } }
    assert_raises(boom) { g.(5) }
    # non-local exit through the native frame
    r = catch(:done) { fn { on("x") { |x| throw :done, x * 2 } }.(21) }
    assert_equal 42, r
    # next inside a body returns from the body
    h = fn { on("x") { |x| next x + 1 } }
    assert_equal 2, h.(1)
    # break from a body proc is a LocalJumpError, as for any proc called later
    assert_raises(LocalJumpError) { fn { on("x") { break } }.(1) }
    # return from the enclosing method through a body
    assert_equal :returned, early_return
    # throw with no catch becomes an UncaughtThrowError, not a crash
    assert_raises(UncaughtThrowError) { fn { on("x") { throw :nope } }.(1) }
    # errors in guards keep their class and message
    err = assert_raises(ZeroDivisionError) { fn { on("x", guard: ->(x) { 1 / x > 0 }) { 1 }; on("_") { 0 } }.(0) }
    assert_includes err.message, "divided by 0"
  end

  def early_return
    fn { on("x") { return :returned } }.(1)
    :not_reached
  end

  def test_deep_recursion_is_not_bounded_by_the_vm_stack
    count = fn(:count) { on("[]") { 0 }; on("(_:xs)") { |xs| 1 + count.(xs) } }
    assert_equal 2000, count.((1..2000).to_a)
    # plain Ruby dies here; stack segments carry the recursion through
    ruby_count = ->(xs) { xs.empty? ? 0 : 1 + ruby_count.(xs[1..]) }
    assert_raises(SystemStackError) { ruby_count.((1..200_000).to_a) }
    assert_equal 200_000, count.((1..200_000).to_a)
    walk = fn(:walk) do
      on("Leaf") { [] }
      on("Node l v r") { |l, v, r| walk.(l) + [v] + walk.(r) }
    end
    deep = (1..50_000).inject(Leaf) { |t, i| Node.new(t, i, Leaf) }
    assert_equal (1..50_000).to_a, walk.(deep)
    # exceptions raised deep inside segments propagate with their class
    boom = Class.new(StandardError)
    bomb = fn(:bomb) { on("0") { raise boom, "bottom" }; on("n") { |n| bomb.(n - 1) + 1 } }
    err = assert_raises(boom) { bomb.(30_000) }
    assert_equal "bottom", err.message
    assert_equal 2000, count.((1..2000).to_a) # depth accounting is intact afterwards
  end

  def test_max_depth_guard
    assert_equal 250_000, HaskellMatch.max_depth
    forever = fn(:forever) { on("n") { |n| forever.(n + 1) + 1 } }
    HaskellMatch.max_depth = 5_000
    err = assert_raises(HaskellMatch::StackOverflowError) { forever.(0) }
    assert_includes err.message, "max_depth (5000)"
    count = fn(:count) { on("[]") { 0 }; on("(_:xs)") { |xs| 1 + count.(xs) } }
    assert_equal 4_000, count.((1..4_000).to_a) # depth accounting is intact
    assert_raises(ArgumentError) { HaskellMatch.max_depth = -1 }
    # tail calls do not count as depth
    loop_fn = fn(:loop_fn) { on("0") { :done }; on("n") { |n| loop_fn.tail(n - 1) } }
    assert_equal :done, loop_fn.(100_000)
  ensure
    HaskellMatch.max_depth = 250_000
  end

  def test_stack_segment_setting
    assert_equal 100, HaskellMatch.stack_segment
    count = fn(:count) { on("[]") { 0 }; on("(_:xs)") { |xs| 1 + count.(xs) } }
    HaskellMatch.stack_segment = 0
    assert_raises(SystemStackError) { count.((1..200_000).to_a) }
    HaskellMatch.stack_segment = 20
    assert_equal 100_000, count.((1..100_000).to_a)
    assert_raises(ArgumentError) { HaskellMatch.stack_segment = -1 }
  ensure
    HaskellMatch.stack_segment = 100
  end

  def test_tail_calls_run_in_constant_stack
    # go acc [] = acc ; go acc (x:xs) = go (acc + x) xs
    go = fn(:go) do
      on("acc", "[]") { |acc| acc }
      on("acc", "(x:xs)") { |acc, x, xs| go.tail(acc + x, xs) }
    end
    assert_equal 500_000 * 500_001 / 2, go.(0, (1..500_000).to_a)
    # tail calls between different functions
    even = odd = nil
    even = fn(:even) { on("0") { true }; on("n") { |n| odd.tail(n - 1) } }
    odd = fn(:odd) { on("0") { false }; on("n") { |n| even.tail(n - 1) } }
    assert even.(1_000_000)
    refute odd.(1_000_000)
    # the marker is an ordinary value outside a call
    assert_equal HaskellMatch::TailCall.new(go, [1, 2], nil), go.tail(1, 2)
    assert_raises(TypeError) { fn { on("x") { HaskellMatch::TailCall.new(5, [1], nil) } }.(1) }
    assert_raises(ArgumentError) { fn { on("x") { HaskellMatch::TailCall.new(go, 1, nil) } }.(1) }
  end

  def test_deferred_calls_keep_continuations_off_the_ruby_stack
    length = fn(:length) do
      on("[]") { 0 }
      on("(_:xs)") { |xs| length.defer(xs) { |n| 1 + n } }
    end
    assert_equal 3, length.([1, 2, 3])
    HaskellMatch.stack_segment = 0 # no segments: continuations alone carry it
    assert_equal 300_000, length.((1..300_000).to_a)
    sum_tree = fn(:sum_tree) do
      on("Leaf") { 0 }
      on("Node l v r") { |l, v, r| sum_tree.defer(l) { |ls| sum_tree.defer(r) { |rs| ls + v + rs } } }
    end
    deep = (1..100_000).inject(Leaf) { |t, i| Node.new(t, i, Leaf) }
    assert_equal 100_000 * 100_001 / 2, sum_tree.(deep)
    assert_raises(ArgumentError) { length.defer([1]) }
  ensure
    HaskellMatch.stack_segment = 100
  end

  def test_recursion_by_name_and_recur
    # the function is in scope inside its own clauses under its name...
    assert_equal 6, fn(:fact) { on("0") { 1 }; on("n") { |n| n * fact.(n - 1) } }.(3)
    # ...and as `recur`, whatever the name
    assert_equal 120, fn { on("0") { 1 }; on("n") { |n| n * recur.(n - 1) } }.(5)
    assert_equal 120, fn(nil) { on("0") { 1 }; on("n") { |n| n * this.(n - 1) } }.(5)
    # mutual recursion through local variables
    even = odd = nil
    even = fn(:even) { on("0") { true }; on("n") { |n| odd.(n - 1) } }
    odd = fn(:odd) { on("0") { false }; on("n") { |n| even.(n - 1) } }
    assert even.(10)
    refute odd.(10)
    # a name that is not a valid method name is simply not defined
    f = fn("not a method name") { on("0") { 1 }; on("n") { |n| recur.(n - 1) } }
    assert_equal 1, f.(3)
    # a clause name clashing with the builder's own methods does not override it
    on = fn(:on) { on("0") { :zero }; on("_") { recur.(0) } }
    assert_equal :zero, on.(5)
  end

  def test_callable_protocols
    inc = fn { on("n") { |n| n + 1 } }
    assert_equal [2, 3], [1, 2].map(&inc)
    assert_equal 2, inc[1]
    assert_equal 2, (inc === 1)
    add = fn { on("a", "b") { |a, b| a + b } }
    assert_equal 3, add.curry[1][2]
    assert_equal 3, add.to_proc.call(1, 2)
    assert_equal 2, add.to_proc.arity
    assert inc.clauses.frozen?
    assert_same inc.to_proc, inc.to_proc
  end

  def test_select_reports_clause_and_bindings
    f = fn { on("Just x", guard: ->(x) { x > 0 }) { 1 }; on("Just x") { 2 }; on("Nothing") { 3 } }
    assert_equal [0, [5]], f.select(Just.new(5))
    assert_equal [1, [-5]], f.select(Just.new(-5))
    assert_equal [2, []], f.select(Nothing)
    partial = fn(exhaustive: false) { on("Just x") { 1 } }
    assert_nil partial.select(Nothing)
  end

  def test_definition_block_can_take_the_builder
    f = HaskellMatch.fn(:explicit) do |b|
      b.on("True") { 1 }
      b.on("False") { 0 }
    end
    assert_equal 1, f.(true)
  end

  def test_anonymous_function_name
    f = fn(nil) { on("x") { |x| x } }
    assert_match(/anonymous function at .*function_test\.rb:\d+/, f.name)
  end

  def test_qualified_constructor_names
    f = fn { on("Maybe.Just x") { |x| x }; on("Maybe.Nothing") { 0 } }
    assert_equal 1, f.(Just.new(1))
  end

  def test_comments_in_patterns
    f = fn { on("Just x -- the value") { |x| x }; on("{- nothing -} Nothing") { 0 } }
    assert_equal 1, f.(Just.new(1))
    assert_equal 0, f.(Nothing)
  end

  def test_nil_and_arbitrary_objects
    f = fn { on("x") { |x| x } }
    assert_nil f.(nil)
    o = Object.new
    assert_same o, f.(o)
    assert_equal({ a: 1 }, f.({ a: 1 }))
    g = fn { on("Just x") { |x| x }; on("Nothing") { 0 } }
    assert_raises(HaskellMatch::TypeMismatchError) { g.(nil) }
    assert_raises(HaskellMatch::TypeMismatchError) { g.(Object.new) }
    assert_raises(HaskellMatch::TypeMismatchError) { g.(Just) }
  end

  def test_subclass_instances_do_not_match
    sub = Class.new(Just)
    f = fn { on("Just x") { |x| x }; on("Nothing") { 0 } }
    assert_raises(HaskellMatch::TypeMismatchError) { f.(sub.new(1)) }
  end

  def test_many_literal_cases
    clauses = (0...50).map { |i| [i.to_s, i * 10] }
    f = HaskellMatch.fn(:table) do |b|
      clauses.each { |lit, result| b.on(lit) { result } }
      b.on("_") { -1 }
    end
    50.times { |i| assert_equal i * 10, f.(i) }
    assert_equal(-1, f.(99))
  end

  def test_tree_dump
    f = fn { on("Just (Just x)") { 1 }; on("Just Nothing") { 2 }; on("Nothing") { 3 } }
    dump = f.decision_tree
    assert_includes dump, "switch slot 0 (Maybe)"
    assert_includes dump, "Just -> slots [1]"
    assert_includes dump, "switch slot 1 (Maybe)"
    assert_includes dump, "=> clause 3"
    # each argument position is examined once per path
    assert_equal 1, dump.scan("switch slot 0 ").size
  end
end
