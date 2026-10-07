# frozen_string_literal: true

require_relative "test_helper"

# Tier 1 of the Haskell front end: operators and fixities, infix
# constructors, pattern/let guards, the small syntax extensions, literal
# escapes, and imports/exports between compiled modules.
class HaskellTier1Test < Minitest::Test
  include TestHelpers

  FIXTURES = File.expand_path("fixtures", __dir__)

  def hs(source, **opts)
    HaskellMatch.haskell(source, **opts)
  end

  def test_user_operators_with_declared_fixities
    m = hs(<<~'HS')
      infixr 5 +++
      infixl 7 `dot`

      (+++) :: [a] -> [a] -> [a]
      xs +++ ys = foldr (:) ys xs

      dot :: [Int] -> [Int] -> Int
      dot xs ys = sum (zipWith (*) xs ys)

      (.*.) :: Int -> Int -> Int
      a .*. b = a * b + 1

      useOps :: Int
      useOps = 2 .*. 3 .*. 4

      joined :: [Int]
      joined = [1] +++ [2] +++ [3]

      dotted :: Int
      dotted = [1,2] `dot` [3,4] + 1

      sections :: [Int]
      sections = map (.*. 10) [1, 2] ++ map (3 .*.) [1] ++ zipWith (.*.) [1] [1]

      localOp :: Int -> Int
      localOp x = x <#> 2
        where a <#> b = a * 100 + b

      twice' :: (Int -> Int) -> Int -> Int
      twice' f = f . f

      (<<<) :: Int -> Int -> Int
      (<<<) = (*)
    HS
    assert_equal 29, m.useOps          # undeclared operators are infixl 9
    assert_equal [1, 2, 3], m.joined
    assert_equal 12, m.dotted          # `dot` (infixl 7) binds tighter than +
    assert_equal [11, 21, 4, 2], m.sections
    assert_equal 402, m.localOp(4)
    assert_equal 7, m.send(:".*.", 2, 3)
    assert_equal 2, m.haskell_function(".*.").arity
    assert_equal 12, m.send(:"<<<", 3, 4)
    assert_equal 9, m.twice_prime(->(x) { x + 4 }, 1)
    assert_raises(HaskellMatch::HaskellSyntaxError) { hs("infixl 10 <+>\nf = 1") }
    assert_raises(HaskellMatch::HaskellSyntaxError) { hs("f x <+> y = 1") }
    assert_raises(HaskellMatch::DefinitionError) { hs("f = 1 <?> 2") } # unknown operator
  end

  def test_infix_constructors
    m = hs(<<~HS)
      infixr 5 :+:
      data C = Int :+: Int | CZero deriving (Eq, Show)

      re :: C -> Int
      re (a :+: _) = a
      re CZero = 0

      mk :: Int -> Int -> C
      mk a b = a :+: b

      mk' :: Int -> Int -> C
      mk' = (:+:)

      swapC :: C -> C
      swapC (a :+: b) = b :+: a
      swapC z = z
    HS
    v = m.mk(3, 4)
    assert_equal 3, m.re(v)
    assert_equal 0, m.re(m::CZero)
    assert_equal "3 :+: 4", v.inspect
    assert_equal m.mk_prime(3, 4), v
    assert_equal m::C::ColonPlusColon.new(3, 4), v
    assert_equal "data C = _ :+: _ | CZero", m::C.inspect
    assert_equal({ a: 3, b: 4 }, m.pattern("(a :+: b)").match(v))
    assert_equal({ a: 3, b: 4 }, m.pattern("(:+:) a b").match(v))
    assert_equal "a :+: _", m.pattern { ColonPlusColon(a, _) }.source
    assert_equal 7, m.fn(:f) { on(ColonPlusColon(a, b)) { |a, b| a + b }; on("CZero") { 0 } }.(v)
    assert_equal m.mk(4, 3), m.swapC(v)
    err = assert_raises(HaskellMatch::NonExhaustiveError) { m.fn(:g) { on("_ :+: _") { 1 } } }
    assert_includes err.message, "CZero"
    # also from Ruby
    t = HaskellMatch.data "Pair2 = Int :*: Int", under: nil
    k = t.constructors.first
    assert_equal ":*:", k.constructor_name
    assert_equal "1 :*: 2", k.new(1, 2).inspect
    assert_equal({ x: 1 }, HaskellMatch.pattern("x :*: _").match(k.new(1, 2)))
  end

  def test_pattern_guards_and_let_guards
    m = hs(<<~HS)
      guards :: [(Int, String)] -> Int -> String
      guards m k
        | Just v <- lookup k m, length v > 3 = "long " ++ v
        | Just v <- lookup k m = v
        | let w = k * 2, w > 10 = "big " ++ show w
        | otherwise = "none"

      caseGuard :: Maybe Int -> Int
      caseGuard v = case v of
        Just n | Just d <- lookup n [(1, 10), (2, 20)] -> d
               | otherwise -> n
        Nothing -> -1

      val :: Int
      val
        | Just x <- lookup 2 [(2, 5)] = x * 2
        | otherwise = 0

      both :: Maybe Int -> Maybe Int -> Int
      both a b
        | Just x <- a, Just y <- b, let s = x + y, s > 0 = s
        | otherwise = 0
    HS
    table = [[1, "ab"], [2, "abcdef"]]
    assert_equal ["ab", "long abcdef", "big 14", "none"], [1, 2, 7, 3].map { |k| m.guards(table, k) }
    assert_equal [10, 5, -1], [Just.new(1), Just.new(5), Nothing].map { |v| m.caseGuard(v) }
    assert_equal 10, m.val
    assert_equal [3, 0, 0], [m.both(Just.new(1), Just.new(2)), m.both(Nothing, Just.new(2)), m.both(Just.new(-5), Just.new(2))]
  end

  def test_multiway_if_lambda_case_and_tuple_sections
    m = hs(<<~'HS')
      classify :: Int -> String
      classify x = if | x < 0 -> "neg"
                      | x == 0 -> "zero"
                      | otherwise -> "pos"

      partial :: Int -> String
      partial x = if | x > 0 -> "pos"

      lc :: [Int] -> [String]
      lc = map (\case
        0 -> "z"
        n | even n -> "e"
          | otherwise -> "o")

      pairUp :: [Int] -> [(Int, String)]
      pairUp = map (,"x")

      triple :: Int -> (Int, Int, Int)
      triple = (1,,3)

      both :: Int -> Int -> (Int, Int)
      both = (,)
    HS
    assert_equal %w[neg zero pos], [-1, 0, 5].map { |x| m.classify(x) }
    err = assert_raises(HaskellMatch::Prelude::HaskellError) { m.partial(-1) }
    assert_includes err.message, "multi-way if"
    assert_equal %w[z e o], m.lc([0, 2, 3]).to_a
    assert_equal [[1, "x"], [2, "x"]], m.pairUp([1, 2]).to_a
    assert_equal [1, 2, 3], m.triple(2)
    assert_equal [1, 2], m.both(1, 2)
  end

  def test_record_selectors_construction_and_update
    m = hs(<<~HS)
      data Pet = Dog { petName :: String, age :: Int } | Cat { petName :: String }
      newtype Wrapper = Wrapper { unwrap :: Int }

      name :: Pet -> String
      name p = petName p

      names :: [Pet] -> [String]
      names = map petName

      birthday :: Pet -> Pet
      birthday d@(Dog {}) = d { age = age d + 1 }
      birthday c = c

      mk :: String -> Int -> Pet
      mk petName age = Dog { petName, age }

      rename :: String -> Pet -> Pet
      rename n p = p { petName = n }
    HS
    rex = m::Dog.new("rex", 3)
    tom = m::Cat.new("tom")
    assert_equal %w[rex tom], [m.name(rex), m.name(tom)]
    assert_equal %w[rex tom], m.names([rex, tom]).to_a
    assert_equal m::Dog.new("rex", 4), m.birthday(rex)
    assert_equal tom, m.birthday(tom)
    assert_equal rex, m.mk("rex", 3)
    assert_equal m::Cat.new("tim"), m.rename("tim", tom)
    assert_equal 5, m.unwrap(m::Wrapper.new(5))
    assert_equal 3, m.age(rex)
    assert_raises(NoMethodError) { m.age(tom) } # Cats have no age, as in Haskell (a runtime error)
    assert_raises(ArgumentError) { hs("data P = P { a :: Int, b :: Int }\nf = P { a = 1 }").f } # missing field
  end

  def test_functions_apply_like_haskell_from_ruby
    m = hs(<<~'HS')
      add3 :: Int -> Int -> Int -> Int
      add3 a b c = a + b + c

      adder :: Int -> (Int -> Int)
      adder n = \x -> x + n

      twice :: (a -> a) -> a -> a
      twice f = f . f
    HS
    assert_equal 6, m.add3(1, 2, 3)                      # saturated
    assert_equal 6, m.add3(1).(2).(3)                    # fewer arguments: a curried partial application
    assert_equal 6, m.add3(1, 2).(3)
    assert_kind_of Proc, m.add3
    assert_equal 6, m.add3.(1).(2).(3)
    assert_equal 15, m.adder(5).(10)                     # a lambda returned from Haskell is a Proc
    assert_equal 15, m.adder(5, 10)                      # extra arguments apply to the result
    assert_equal "hi!!", m.twice(->(s) { s + "!" }, "hi") # Ruby lambdas are Haskell functions
    assert_equal "hi!!", m.twice(->(s) { s + "!" }).("hi")
    assert_equal [2, 3], [1, 2].map(&m.adder(1))
    assert_equal 6, m.haskell_function(:add3).curried.(1).(2).(3)
    assert_equal 6, m.haskell_function(:add3).to_proc.(1, 2, 3)
  end

  def test_literal_escapes_and_numeric_forms
    m = hs(<<~'HS')
      esc :: String
      esc = "tab\tnl\\\&x\65\x42\o103\SOH\^A\
            \ end"

      chars :: [Char]
      chars = ['\'', '\DEL', '\n', 'a']

      nums :: [Double]
      nums = [0x10, 0o17, 0b101, 1.5e3, 2E-2, 1_000]
    HS
    assert_equal "tab\tnl\\xABC\u0001\u0001 end", m.esc
    assert_equal ["'", "\u007f", "\n", "a"], m.chars
    assert_equal [16, 15, 5, 1500.0, 0.02, 1000], m.nums
    assert_raises(HaskellMatch::HaskellSyntaxError) { hs('s = "\q"') }
  end

  def test_imports_between_compiled_modules
    $LOAD_PATH.unshift(FIXTURES)
    physics = hs(<<~HS)
      module Physics where
      import Vectors
      import qualified Data.Char as Ch (toUpper)
      import Math (hypot)
      import Geo.Angles

      move :: Vec -> Vec -> Vec
      move p v = p <+> scale 2 v

      len :: Vec -> Double
      len (x :| y) = hypot x y

      shout :: String -> String
      shout = map toUpper

      right :: Double
      right = degrees (pi / 2)
    HS
    vec = Vectors::ColonBar
    assert_equal vec.new(3.0, 5.0), physics.move(vec.new(1.0, 1.0), vec.new(1.0, 2.0))
    assert_equal 5.0, physics.len(vec.new(3.0, 4.0))
    assert_equal 5.0, physics.norm(vec.new(3.0, 4.0)) # re-exported forwarding method
    assert_equal 42, physics.hidden
    assert_equal "HI", physics.shout("hi")
    assert_in_delta 90.0, physics.right
    assert_same Geo::Angles, HaskellMatch.require("Geo.Angles") # loaded once, nested constant
    assert_equal %w[Vec <+> scale norm hidden], Vectors.haskell_exports
    assert_equal({ "<+>" => :function, "scale" => :function, "norm" => :function, "hidden" => :value,
                   "hypot" => :function, "degrees" => :function }, physics.haskell_imports)
    assert_equal ["Vec"], Vectors.haskell_types
    refute physics.respond_to?(:secret) # not exported
    assert_equal 7, Vectors.secret # but Ruby can still reach it

    only_norm = hs("import Vectors (norm)\nf = norm", into: Module.new)
    assert_equal 1.0, only_norm.f.(vec.new(0.0, 1.0))
    refute only_norm.respond_to?(:scale)
    without = hs("import Vectors hiding (hidden)\nf = scale 2", into: Module.new)
    assert_equal vec.new(2.0, 4.0), without.f.(vec.new(1.0, 2.0))
    refute without.respond_to?(:hidden)

    err = assert_raises(HaskellMatch::HaskellSyntaxError) { hs("import Vectors (nope)\nf = 1") }
    assert_includes err.message, "does not export nope"
    err = assert_raises(HaskellMatch::HaskellSyntaxError) { hs("import NoSuch\nf = 1") }
    assert_includes err.message, "cannot find module NoSuch"
    err = assert_raises(HaskellMatch::HaskellSyntaxError) { hs("import Data.List (frobnicate)\nf = 1") }
    assert_includes err.message, "does not export frobnicate"
    # a plain Ruby module is importable too
    assert_equal 5.0, hs("import Math (hypot)\nh = hypot 3 4").h
  ensure
    $LOAD_PATH.delete(FIXTURES)
  end

  def test_imported_types_match_in_patterns_and_exhaustiveness
    $LOAD_PATH.unshift(FIXTURES)
    m = hs(<<~HS)
      import Vectors (Vec(..))

      isUnit :: Vec -> Bool
      isUnit (1 :| 0) = True
      isUnit (0 :| 1) = True
      isUnit _ = False
    HS
    assert_equal true, m.isUnit(Vectors::ColonBar.new(1, 0))
    assert_equal false, m.isUnit(Vectors::ColonBar.new(2, 0))
    assert_same Vectors::Vec, m::Vec
    assert_raises(HaskellMatch::NonExhaustiveError) { hs("import Vectors\nf (1 :| _) = 1") }
  ensure
    $LOAD_PATH.delete(FIXTURES)
  end
end
