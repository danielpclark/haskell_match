# frozen_string_literal: true

require_relative "test_helper"

# The "more Ruby" features: Hash patterns, Ruby classes as constructors,
# deriving, checked field types, where helpers, composition and carets.
class RubyTrackTest < Minitest::Test
  include TestHelpers

  # ------------------------------------------------------------ Hash patterns

  def test_hash_patterns_match_keys_and_ignore_the_rest
    greet = fn(:greet) do
      on({ name: n, title: t }) { |n, t| "#{t} #{n}" }
      on({ name: n }) { |n| "hi #{n}" }
      on({}) { "anonymous" }
    end
    assert_equal "Dr Al", greet.(name: "Al", title: "Dr")
    assert_equal "hi Al", greet.(name: "Al", age: 3)
    assert_equal "anonymous", greet.({ other: 1 })
    assert_equal " Al", greet.(name: "Al", title: nil) # a key with a nil value is present
    err = assert_raises(HaskellMatch::TypeMismatchError) { greet.(5) }
    assert_includes err.message, "expected a value of type Hash"
  end

  def test_hash_patterns_quoted_nested_lazy_and_string_keys
    assert_equal({ n: 1, v: 2 }, HaskellMatch.pattern('{name = n, "k" = v}').match({ name: 1, "k" => 2 }))
    assert_nil HaskellMatch.pattern("{name = n}").match({ nome: 1 })
    assert_equal({ i: 7 }, HaskellMatch.pattern { { user: { id: i } } }.match({ user: { id: 7 } }))
    assert_equal({ x: 9 }, HaskellMatch.pattern { lazy({ a: x }) }.match({ a: 9 }))
    assert_equal '{:"odd key" = v, "s" = 1, k = Just x}',
                 HaskellMatch.pattern { { "odd key": v, "s" => 1, k: Just(x) } }.source
    assert_equal({ x: 1 }, HaskellMatch.pattern("{k = Just x}").match({ k: Just.new(1) }))
    assert_equal({ a: 1, b: 2 }, HaskellMatch.case_of({ a: 1, b: 2 }) { on({ a: a, b: b }) { |a, b| { a: a, b: b } }; on(_) { nil } })
  end

  def test_hash_patterns_are_checked_for_exhaustiveness_and_redundancy
    err = assert_raises(HaskellMatch::NonExhaustiveError) { fn { on({ a: x }) { |x| x } } }
    assert_includes err.message, "p1 where p1 is a Hash without the key a"
    err = assert_raises(HaskellMatch::NonExhaustiveError) { fn { on({ a: 1 }) { 1 }; on({ b: _ }) { 2 } } }
    assert_includes err.message, "{a = p1} where p1 is not one of {1}"
    assert_includes err.message, "p1 where p1 is a Hash without the key a or without the key b"
    err = assert_raises(HaskellMatch::RedundantClauseError) { fn { on({ a: _ }) { 1 }; on({ a: _, b: _ }) { 2 }; on(_) { 3 } } }
    assert_includes err.message, "f {a = _, b = _} = ..."
    # `{}` covers every Hash; a different key set is not redundant
    fn { on({ a: _ }) { 1 }; on({}) { 2 } }
    fn { on({ a: _ }) { 1 }; on({ b: _ }) { 2 }; on(_) { 3 } }
    # a Hash column cannot be mixed with constructors
    assert_raises(HaskellMatch::PatternTypeError) { fn { on({ a: _ }) { 1 }; on(Just(_)) { 2 } } }
    assert_includes fn(:h) { on({ a: _ }) { 1 }; on(_) { 2 } }.decision_tree, "hash slot 0 has keys {a}"
  end

  # ------------------------------------------------------- Ruby classes as constructors

  SCircle = Data.define(:r)
  SRect = Data.define(:w, :h)
  class STri
    attr_reader :a, :b, :c

    def initialize(a, b, c)
      @a = a
      @b = b
      @c = c
    end
  end

  def test_sealed_makes_ruby_classes_the_constructors_of_a_type
    shape = HaskellMatch.sealed(:SShape, SCircle, SRect, STri => %i[a b c])
    area = fn(:area) do
      on(SCircle(r)) { |r| 3 * r * r }
      on("SRect w h") { |w, h| w * h }
      on("STri { a = a, b = b, c = c }") { |a, b, c| a + b + c }
    end
    assert_equal [3, 6, 12], [area.(SCircle.new(1)), area.(SRect.new(2, 3)), area.(STri.new(3, 4, 5))]
    assert_equal({ x: 2 }, HaskellMatch.pattern("SCircle { r = x }").match(SCircle.new(2)))
    err = assert_raises(HaskellMatch::NonExhaustiveError) { fn { on(SCircle(_)) { 1 } } }
    assert_includes err.message, "SRect _ _"
    assert_includes err.message, "STri _ _ _"
    assert shape === SCircle.new(1)
    refute shape === 5
    assert_equal [SCircle, SRect, STri], shape.constructors
    assert_equal "data SShape = SCircle {r} | SRect {w, h} | STri {a, b, c}", shape.inspect
    assert_same shape, SCircle.data_type
    assert_equal "SCircle", SCircle.constructor_name
    assert_equal %i[a b c], STri.field_names
    assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.sealed(:Again, SCircle) }
    assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.sealed(:Plain, Class.new) }
    assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.sealed(:NoFields, String) }
  end

  Point2 = Data.define(:x, :y)
  Pair2 = Struct.new(:l, :r)

  def test_data_and_struct_classes_match_without_a_declaration
    # Point2 and Pair2 are constants of this test class: visible from the block
    assert_equal 3, fn { on(Point2(x, y)) { |x, y| x + y } }.(Point2.new(1, 2))
    assert_equal({ l: 1, r: 2 }, HaskellMatch.pattern { Pair2(l, r) }.match(Pair2.new(1, 2)))
    assert_equal({ l: 1 }, HaskellMatch.pattern("Pair2 { l = l }").match(Pair2.new(1, 2))) # registered above
    assert_equal :origin, HaskellMatch.case_of(Point2.new(0, 0)) { on(Point2(0, 0)) { :origin }; on(_) { :other } }
    # a single-constructor type: one clause is exhaustive, a literal needs a fallback
    assert_raises(HaskellMatch::NonExhaustiveError) { fn { on(Point2(0, _)) { 0 } } }
    assert_raises(HaskellMatch::UnknownConstructorError) { fn { on("NoSuchThing x") { 1 } } }
  end

  # ------------------------------------------------------------------ deriving

  def test_deriving_ord_enum_and_bounded
    color = HaskellMatch.data "Hue = Red2 | Green2 | Blue2 deriving (Eq, Ord, Enum, Bounded, Show)", under: nil
    red, green, blue = color.constructors
    assert red < blue
    assert_equal [red, green, blue], [blue, red, green].sort
    assert_equal green, red.succ
    assert_equal green, blue.pred
    assert_equal [red, green, blue], (red..blue).to_a
    assert_equal [green, blue], color.enum_from(green)
    assert_equal [red, green], color.enum_from_to(red, green)
    assert_equal [red, blue], color.enum_from_then(red, blue)
    assert_equal [blue, green, red], color.enum_from_then_to(blue, green, red)
    assert_equal [red, blue], color.enum_from_then_to(red, blue, blue)
    assert_equal [red, blue], [color.min_bound, color.max_bound]
    assert_equal green, color.to_enum(1)
    assert_equal 1, green.from_enum
    assert_equal [red, green, blue], color.values
    assert_raises(ArgumentError) { blue.succ }
    assert_raises(ArgumentError) { red.pred }
    assert_nil(red <=> Just.new(1))

    pt = HaskellMatch.data "Pt2 = Pt2 Int Int deriving (Ord)", under: nil
    k = pt.constructors.first
    assert k.new(1, 2) < k.new(1, 3)
    assert_equal k.new(1, 9), [k.new(2, 0), k.new(1, 9)].min
    assert_equal k.new(1, 2), k.new(1, 2)

    err = assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.data "BadEnum = A9 Int | B9 deriving Enum", under: nil }
    assert_includes err.message, "must be an enumeration type"
    assert_nil HaskellMatch.constructor("A9") # nothing was registered
    err = assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.data "BadCls = A9 deriving Generic", under: nil }
    assert_includes err.message, "cannot derive 'Generic'"
    # the Hash form takes deriving: too
    t = HaskellMatch.data(:Sz, Small: 0, Large: 0, deriving: %i[Ord Enum], under: nil)
    assert_equal t.constructors, t.enum_from(t.constructors.first)
  end

  # --------------------------------------------------------- checked field types

  def test_check_types_validates_constructor_arguments
    person = HaskellMatch.data "Person2 = Person2 { name :: String, age :: Int, tags :: [String], boss :: Maybe Person2, f :: Int -> Int, pos :: (Int, Double), any :: a }",
                               under: nil, check_types: true
    k = person.constructors.first
    idf = ->(x) { x }
    ok = k.new("Al", 3, [], Nothing, idf, [1, 2.0], :anything)
    assert_equal "Al", ok.name
    assert_equal ok, k.new(name: "Al", age: 3, tags: [], boss: Nothing, f: idf, pos: [1, 2.0], any: :anything)
    bad = lambda do |**over|
      args = { name: "Al", age: 3, tags: [], boss: Nothing, f: ->(x) { x }, pos: [1, 2.0], any: 1 }.merge(over)
      assert_raises(HaskellMatch::FieldTypeError) { k.new(**args) }
    end
    assert_includes bad.(age: "3").message, "field 'age' expects Int, got \"3\" (String)"
    bad.(name: 3)
    bad.(tags: 5)
    bad.(boss: 5)
    k.new(**{ name: "Al", age: 3, tags: [], boss: Just.new(1), f: idf, pos: [1, 2.0], any: 1 }) # any Maybe passes
    bad.(f: 5)
    bad.(pos: [1])
    bad.(pos: [1, "x"])
    assert_raises(ArgumentError) { k.new("Al") } # missing fields are still Data's business

    money = HaskellMatch.data(:Money2, Money2: { amount: :Integer, currency: :Symbol }, check_types: true, under: nil)
    m = money.constructors.first
    assert_equal :usd, m.new(5, :usd).currency
    assert_raises(HaskellMatch::FieldTypeError) { m.new(5, "usd") }
    assert_raises(HaskellMatch::FieldTypeError) { m.new(5.0, :usd) }

    # positional constructors are checked too; unchecked by default
    shape = HaskellMatch.data "Shape2 = Circle2 Double | Rect2 Double Double", under: nil, check_types: true
    c, r = shape.constructors
    assert_equal 1, c.new(1).r rescue assert_equal 1, c.new(1)._1
    assert_raises(HaskellMatch::FieldTypeError) { r.new(1, "2") }
    loose = HaskellMatch.data "Shape3 = Circle3 Double", under: nil
    assert_equal "x", loose.constructors.first.new("x")._1

    # a Ruby class or module named as a type works as a type
    t = HaskellMatch.data "Stamp = Stamp Time Hash", under: nil, check_types: true
    s = t.constructors.first
    assert s.new(Time.now, {})
    assert_raises(HaskellMatch::FieldTypeError) { s.new(1, {}) }
    assert_kind_of HaskellMatch::Error, HaskellMatch::FieldTypeError.new
  end

  def test_check_field_types_global_default
    HaskellMatch.check_field_types = true
    t = HaskellMatch.data "Checked = Checked Int", under: nil
    assert_raises(HaskellMatch::FieldTypeError) { t.constructors.first.new("x") }
  ensure
    HaskellMatch.check_field_types = false
  end

  # ------------------------------------------------------------- where helpers

  def test_where_helpers_are_local_checked_functions
    sum_to = fn(:sum_to) do
      on(n) { |n| go.(n, 0) }
      where :go do
        on(0, acc) { |acc| acc }
        on(k, acc) { |k, acc| go.tail(k - 1, acc + k) }
      end
    end
    assert_equal 500_000_500_000, sum_to.(1_000_000)
    assert_equal %w[go], sum_to.helpers.keys
    assert_kind_of HaskellMatch::Function, sum_to.helpers["go"]
    # helpers see each other and the enclosing function
    parity = fn(:parity) do
      on(n) { |n| ev.(n) ? :even : :odd }
      where(:ev) { on(0) { true }; on(n) { |n| od.(n - 1) } }
      where(:od) { on(0) { false }; on(n) { |n| ev.(n - 1) } }
    end
    assert_equal %i[even odd], [parity.(10), parity.(7)]
    # exhaustiveness and options apply to helpers, with overrides
    err = assert_raises(HaskellMatch::NonExhaustiveError) { fn { on(n) { |n| g.(n) }; where(:g) { on(0) { 0 } } } }
    assert_includes err.message, "In an equation for 'g'"
    fn(exhaustive: :ignore) { on(n) { |n| g.(n) }; where(:g) { on(0) { 0 } } }
    fn { on(n) { |n| g.(n) }; where(:g, exhaustive: :ignore) { on(0) { 0 } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on(n) { |n| n }; where(:g) { on(_) { 0 } }; where(:g) { on(_) { 1 } } } }
    assert_raises(HaskellMatch::DefinitionError) { fn { on(n) { |n| n }; where(:Bad) { on(_) { 0 } } } }
  end

  def test_where_helpers_in_ractor_mode
    skip "Ractor unavailable" unless defined?(Ractor)

    r = fn(:rsum, ractor: true) do
      on(n) { |n| go.(n, 0) }
      where :go do
        on(0, acc) { |acc| acc }
        on(k, acc) { |k, acc| go.tail(k - 1, acc + k) }
      end
    end
    assert_equal 5050, r.(100)
    assert_equal 5050, quietly { Ractor.new(r) { |f| f.(100) }.take }
  end

  # --------------------------------------------------------------- composition

  def test_functions_compose_like_procs
    double = fn(:double) { on(x) { |x| x * 2 } }
    inc = ->(x) { x + 1 }
    assert_equal 11, (double >> inc).(5)
    assert_equal 12, (double << inc).(5)
    assert_equal 22, (double >> double).(5) + 2
    assert_equal 12, (inc >> double).(5)
  end

  # -------------------------------------------------------------------- carets

  def test_syntax_errors_show_the_source_and_a_caret
    err = assert_raises(HaskellMatch::PatternSyntaxError) { fn { on("Just (x") { 1 } } }
    assert_equal ["clause 1: expected ')' but reached end of pattern", "    Just (x", "          ^"], err.message.lines.map(&:chomp)
    err = assert_raises(HaskellMatch::PatternSyntaxError) { HaskellMatch.pattern("[a, ") }
    assert_match(/\n    \[a, \n\s+\^\z/, err.message)
    err = assert_raises(HaskellMatch::HaskellSyntaxError) { HaskellMatch.haskell("f x =\n  | x") }
    lines = err.message.lines.map(&:chomp)
    assert_match(/:\d+:5: unexpected pipe in expression\z/, lines[0])
    assert_equal ["    | x", "      ^"], lines[1..]
  end

  private

  def quietly
    old = $stderr
    $stderr = StringIO.new
    yield
  ensure
    $stderr = old
  end
end
