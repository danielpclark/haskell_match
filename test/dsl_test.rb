# frozen_string_literal: true

require_relative "test_helper"

class DslTest < Minitest::Test
  include TestHelpers

  class Account
    extend HaskellMatch::DSL
    include Maybe

    attr_reader :balance

    def initialize(balance)
      @balance = balance
    end

    hdef :apply do
      on("Nothing") { self }
      on("Just amount", guard: ->(amount) { amount <= limit }) { |amount| Account.new(balance + amount) }
      on("Just amount") { |amount| raise ArgumentError, "over limit: #{amount} > #{limit}" }
    end

    def limit
      100
    end

    hdef :describe do
      on("x", "y") { |x, y| "#{x}/#{y} for #{balance}" }
    end
  end

  def test_hdef_defines_instance_methods_with_self
    a = Account.new(10)
    assert_same a, a.apply(Nothing)
    assert_equal 60, a.apply(Just.new(50)).balance
    assert_raises(ArgumentError) { a.apply(Just.new(500)) }
    assert_equal "1/2 for 10", a.describe(1, 2)
    assert_equal 1, Account.instance_method(:apply).arity.abs
  end

  def test_hdef_on_object_singleton
    o = Object.new
    o.extend(HaskellMatch::DSL)
    o.instance_variable_set(:@tag, "t")
    o.hdef(:tagged) { on("Just x") { |x| "#{@tag}#{x}" }; on("Nothing") { @tag } }
    assert_equal "t1", o.tagged(Just.new(1))
    assert_equal "t", o.tagged(Nothing)
  end

  def test_hdef_enforces_exhaustiveness_and_arity
    assert_raises(HaskellMatch::NonExhaustiveError) do
      Class.new { extend HaskellMatch::DSL; hdef(:bad) { on("Just x") { 1 } } }
    end
    k = Class.new { extend HaskellMatch::DSL; hdef(:partial, exhaustive: false) { on("Just x") { |x| x } } }
    assert_raises(HaskellMatch::MatchError) { k.new.partial(Nothing) }
    assert_raises(ArgumentError) { k.new.partial }
    assert_raises(ArgumentError) { Class.new { extend HaskellMatch::DSL; hdef(:nope) } }
  end

  def test_dsl_delegates
    o = Object.new.extend(HaskellMatch::DSL)
    f = o.fn(:inc) { on("n") { |n| n + 1 } }
    assert_equal 2, f.(1)
    assert_equal 3, o.case_of(Just.new(3)) { on("Just x") { |x| x }; on("Nothing") { 0 } }
    assert_equal({ x: 1 }, o.pattern("Just x").match(Just.new(1)))
    t = o.data("DslType = D Int")
    assert_equal "D 1", t::D.new(1).inspect
  end
end
