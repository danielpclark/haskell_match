# frozen_string_literal: true

require_relative "test_helper"

class DataTest < Minitest::Test
  include TestHelpers

  def test_string_declaration_defines_module_and_constructors
    assert_kind_of Module, Maybe
    assert_equal "Maybe", Maybe.type_name
    assert_equal ["a"], Maybe.type_variables
    assert_equal %w[Nothing Just], Maybe.constructor_names
    assert_equal [Nothing, Just], Maybe.constructors
    assert_equal Maybe, Just.data_type
    assert_equal "Maybe::Just", Just.name
    assert_equal "Maybe::Nothing", Nothing.class.name
  end

  def test_constructors_with_fields_are_data_classes
    v = Just.new(5)
    assert_kind_of Data, v
    assert_equal [5], v.fields
    assert_equal 5, v[0]
    assert_equal 5, v._1
    assert v.frozen?
    assert_equal Just.new(5), Just[5]
    assert_equal Just.new(5), Just.(5)
    assert_equal Just.new(5), [5].map(&Just).first
    assert_equal 1, Just.arity
    refute Just.record?
    refute Just.nullary?
    assert_equal "Just", v.constructor_name
    assert_equal Maybe, v.data_type
  end

  def test_nullary_constructors_are_singleton_values
    assert Nothing.frozen?
    assert_equal Nothing, Maybe::Nothing
    assert Nothing.class.nullary?
    assert_equal Nothing, Nothing.class.value
    assert_raises(NoMethodError) { Nothing.class.new }
    assert_equal [], Nothing.fields
    assert_same Nothing, Nothing.class.value
  end

  def test_equality_and_hash
    assert_equal Just.new(1), Just.new(1)
    refute_equal Just.new(1), Just.new(2)
    refute_equal Just.new(1), Left.new(1)
    assert_equal Just.new(1).hash, Just.new(1).hash
    assert Just.new(1).eql?(Just.new(1))
    assert_equal 1, { Just.new(1) => :a, Just.new(1) => :b }.size
    assert_equal Node.new(Leaf, 1, Leaf), Node.new(Leaf, 1, Leaf)
  end

  def test_haskell_style_inspect
    assert_equal "Nothing", Nothing.inspect
    assert_equal "Just 1", Just.new(1).inspect
    assert_equal "Just (Just 1)", Just.new(Just.new(1)).inspect
    assert_equal "Just (-1)", Just.new(-1).inspect
    assert_equal "Just [1, 2]", Just.new([1, 2]).inspect
    assert_equal "Just \"s\"", Just.new("s").inspect
    assert_equal "Rect 1 2.5", Rect.new(1, 2.5).inspect
    assert_equal "Node Leaf 1 (Node Leaf 2 Leaf)", Node.new(Leaf, 1, Node.new(Leaf, 2, Leaf)).inspect
    assert_equal "Person {name = \"Ann\", age = 30}", Person.new("Ann", 30).inspect
    assert_equal "Just True", Just.new(true).inspect
    assert_equal "Just 1", Just.new(1).to_s
    assert_equal "data Maybe a = Nothing | Just _", Maybe.inspect
    # `include Person` makes `Person` the constructor; the type is `::Person`
    assert_equal "data Person = Person {name, age}", ::Person.inspect
    assert_equal ::Person, Person.data_type
  end

  def test_records
    p = Person.new(name: "Ann", age: 30)
    assert_equal "Ann", p.name
    assert_equal 30, p.age
    assert_equal p, Person.new("Ann", 30)
    assert_equal %i[name age], Person.field_names
    assert Person.record?
    assert_equal({ name: "Ann", age: 30 }, p.to_h)
    assert_equal 31, p.with(age: 31).age
    assert_equal "Ann", p[:name]
  end

  def test_type_module_case_equality
    assert Maybe === Just.new(1)
    assert Maybe === Nothing
    refute Maybe === Left.new(1)
    refute Maybe === 5
    result = case Just.new(1)
             when Either then :either
             when Maybe then :maybe
             end
    assert_equal :maybe, result
  end

  def test_include_brings_constructors_into_scope
    mod = Module.new do
      include Maybe
      def self.wrap(x)
        Just.new(x)
      end
    end
    assert_equal Just.new(3), mod.wrap(3)
  end

  def test_hash_declaration_forms
    t = HaskellMatch.data(:HashForm, Zero: 0, One: 1, Two: [:a, :b], Rec: { x: :Int, y: :Int }, Unit: nil)
    assert_equal %w[Zero One Two Rec Unit], t.constructor_names
    assert_equal 2, HashForm::Two.arity
    assert_equal %i[x y], HashForm::Rec.field_names
    assert HashForm::Zero.class.nullary?
    assert HashForm::Unit.class.nullary?
    assert_equal "Rec {x = 1, y = 2}", HashForm::Rec.new(1, 2).inspect
    f = fn { on("Two a b") { |a, b| a + b }; on("Rec { x, y }") { |x, y| x * y }; on("_") { 0 } }
    assert_equal 3, f.(HashForm::Two.new(1, 2))
    assert_equal 6, f.(HashForm::Rec.new(2, 3))
    assert_equal 0, f.(HashForm::Zero)
  end

  def test_under_option
    ns = Module.new
    t = HaskellMatch.data("Inner = In Int", under: ns)
    assert_same t, ns::Inner
    refute Object.const_defined?(:Inner)
    anon = HaskellMatch.data("Anon = An Int", under: nil)
    assert_same anon, HaskellMatch::Types::Anon
    assert_equal "HaskellMatch::Types::Anon::An", anon::An.name
    assert_equal "An 1", anon::An.new(1).inspect
  end

  def test_redefinition_replaces_type
    HaskellMatch.data "Redef = A1 | B1 Int"
    old = Redef
    f = fn { on("A1") { :a }; on("B1 _") { :b } }
    assert_equal :a, f.(Redef::A1)
    HaskellMatch.data "Redef = A1 | B1 Int | C1"
    refute_same old, Redef
    assert_equal %w[A1 B1 C1], Redef.constructor_names
    # the old compiled function keeps working with values of the old type
    assert_equal :b, f.(old::B1.new(1))
    # and new definitions see the new constructors
    err = assert_raises(HaskellMatch::NonExhaustiveError) { fn { on("A1") { 1 }; on("B1 _") { 2 } } }
    assert_includes err.message, "C1"
  end

  def test_declaration_errors
    assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.data("lower = X") }
    assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.data("NoCons") }
    assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.data("Dup = A | A") }
    assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.data("Bool = Yes | No") }
    assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.data(:Empty) }
    assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.data(:Neg, A: -1) }
    assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.data(:BadField, R: { Upper: :Int }) }
    assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.data(:BadSpec, A: "x") }
    assert_raises(ArgumentError) { HaskellMatch.data("X = Y", Z: 0) }
    # constructor clash with another live type
    err = assert_raises(HaskellMatch::DataDeclarationError) { HaskellMatch.data("Clash = Just Int") }
    assert_includes err.message, "already defined by type 'Maybe'"
    refute Object.const_defined?(:Clash)
  end

  def test_deriving_and_strictness_are_accepted
    t = HaskellMatch.data("Strict = S !Int {-# UNPACK #-} !Double deriving (Show, Eq)")
    assert_equal 2, t::S.arity
  end

  def test_data_values_support_ruby_pattern_matching_too
    case Just.new(5)
    in { _1: Integer => n }
      assert_equal 5, n
    end
    case Person.new("Ann", 30)
    in { name:, age: }
      assert_equal ["Ann", 30], [name, age]
    end
  end
end
