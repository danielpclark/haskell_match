# frozen_string_literal: true

require_relative "test_helper"

class HaskellTest < Minitest::Test
  include TestHelpers

  EXAMPLES = File.expand_path("../examples", __dir__)

  def hs(source, **opts)
    HaskellMatch.haskell(source, **opts)
  end

  # ------------------------------------------------------------ basics

  def test_data_equations_and_guards
    m = hs(<<~HS)
      data Shape = Circle Double | Rect Double Double

      area :: Shape -> Double
      area (Circle r) = 3 * r * r
      area (Rect w h) = w * h

      describe :: Int -> String
      describe n
        | n < 0 = "negative"
        | n == 0 = "zero"
        | otherwise = "positive"
    HS
    assert_equal 12.0, m.area(m::Circle.new(2.0))
    assert_equal 6.0, m.area(m::Rect.new(2.0, 3.0))
    assert_equal %w[negative zero positive], [-3, 0, 9].map { |n| m.describe(n) }
    assert_kind_of HaskellMatch::Function, m.haskell_function(:area)
    assert_equal %w[area describe], m.haskell_functions.keys
    assert_raises(NameError) { m.haskell_function(:nope) }
  end

  def test_camel_case_names_get_snake_case_aliases_and_primes_become_prime
    m = hs(<<~HS)
      sumTo :: Int -> Int
      sumTo n = go n 0
        where
          go 0 acc = acc
          go k acc = go (k - 1) (acc + k)

      signum' :: Int -> Int
      signum' (-1) = -1
      signum' 0 = 0
      signum' _ = 1
    HS
    assert_equal 5050, m.sumTo(100)
    assert_equal 5050, m.sum_to(100)
    assert_equal [-1, 0, 1], [-1, 0, 7].map { |n| m.signum_prime(n) }
    assert_equal 1, m.send(:"signum'", 3)
  end

  def test_where_helpers_are_tail_recursive_without_stack_limits
    m = hs(<<~HS)
      sumTo :: Int -> Int
      sumTo n = go n 0
        where
          go 0 acc = acc
          go k acc = go (k - 1) (acc + k)

      loop :: Int -> Int -> Int
      loop 0 acc = acc
      loop n acc = loop (n - 1) (acc + 1)
    HS
    assert_equal 500_000_500_000, m.sumTo(1_000_000)
    assert_equal 1_000_000, m.loop(1_000_000, 0)
  end

  def test_where_and_let_capture_enclosing_variables
    m = hs(<<~HS)
      scale :: Int -> [Int] -> [Int]
      scale k xs = map times xs
        where times x = x * k

      nested :: Int -> Int
      nested n = outer n
        where
          outer x = inner x + 1
            where inner y = y * n

      letFun :: Int -> Int
      letFun x = let double y = y * 2
                     add a b = a + b
                 in add (double x) x

      letVals :: Int -> Int
      letVals x = let y = x * 2
                      z = y + 1
                  in y + z

      whereVals :: Int -> Int
      whereVals x = a + b
        where
          a = x * 10
          (b, _) = (x, x)

      anyOrder :: Int -> Int
      anyOrder x = c
        where
          c = b * 2
          b = a + 1
          a = x
    HS
    assert_equal [3, 6, 9], m.scale(3, [1, 2, 3])
    assert_equal 17, m.nested(4)
    assert_equal 15, m.letFun(5)
    assert_equal 21, m.letVals(5)
    assert_equal 22, m.whereVals(2)
    assert_equal 12, m.anyOrder(5)
  end

  def test_case_if_lambdas_sections_and_composition
    m = hs(<<~'HS')
      classify :: Int -> String
      classify n = case compare n 10 of
        LT -> "small"
        EQ -> "ten"
        GT -> "big"

      caseGuard :: Int -> String
      caseGuard n = case n of
        0 -> "zero"
        k | k < 0 -> "neg"
          | even k -> "even"
          | otherwise -> "odd"

      ifDemo :: Int -> String
      ifDemo n = if n > 5 then "big" else if n > 2 then "mid" else "small"

      twoArgs :: [Int] -> [Int] -> [Int]
      twoArgs = zipWith (\a b -> a * 10 + b)

      lam :: [Int] -> [Int]
      lam xs = map (\x -> x + 100) xs

      compose :: [Int] -> [Int]
      compose = map (*2) . filter (>3)

      halve :: Int -> Int
      halve = (`div` 2)

      sub1 :: Int -> Int
      sub1 = subtract 1

      dollar :: Int -> Int
      dollar x = negate $ x + 1

      applyTwice :: (a -> a) -> a -> a
      applyTwice f x = f (f x)
    HS
    assert_equal %w[small ten big], [3, 10, 99].map { |n| m.classify(n) }
    assert_equal %w[zero neg even odd], [0, -2, 4, 7].map { |n| m.caseGuard(n) }
    assert_equal %w[small mid big], [1, 3, 9].map { |n| m.ifDemo(n) }
    assert_equal [13, 24], m.twoArgs([1, 2], [3, 4])
    assert_equal [101, 102], m.lam([1, 2])
    assert_equal [8, 10, 12], m.compose([1, 2, 3, 4, 5, 6])
    assert_equal 4, m.halve(9)
    assert_equal 8, m.sub1(9)
    assert_equal(-5, m.dollar(4))
    assert_equal 2, m.applyTwice(->(x) { x + 1 }, 0)
  end

  def test_strings_are_lists_of_characters
    m = hs(<<~HS)
      isHello :: String -> Bool
      isHello "hello" = True
      isHello _ = False

      vowel :: Char -> Bool
      vowel 'a' = True
      vowel 'e' = True
      vowel _ = False

      caps :: String -> String
      caps [] = []
      caps (c:cs) = toUpper c : cs

      len :: [a] -> Int
      len [] = 0
      len (_:xs) = 1 + len xs

      strs :: [String] -> String
      strs = unwords . map reverse

      showIt :: Int -> String
      showIt n = "n=" ++ show n
    HS
    assert_equal true, m.isHello("hello")
    assert_equal false, m.isHello("nope")
    assert_equal [true, false], [m.vowel("a"), m.vowel("z")]
    assert_equal "Word", m.caps("word")
    assert_equal 5, m.len("hello")
    assert_equal "cba ed", m.strs(%w[abc de])
    assert_equal "n=3", m.showIt(3)
  end

  def test_tuples_maybe_and_prelude_lookups
    m = hs(<<~HS)
      tupled :: (Int, Int) -> Int
      tupled (a, b) = a + b

      swap' :: (a, b) -> (b, a)
      swap' (a, b) = (b, a)

      safeDiv :: Int -> Int -> Maybe Int
      safeDiv _ 0 = Nothing
      safeDiv a b = Just (a `div` b)

      unwrap :: Maybe Int -> Int
      unwrap (Just v) = v
      unwrap Nothing = -1

      lookupOr :: Int -> [(Int, String)] -> String
      lookupOr k kvs = fromMaybe "?" (lookup k kvs)

      pairs :: [(Int, Char)]
      pairs = zip [1,2,3] "abc"
    HS
    assert_equal 5, m.tupled([2, 3])
    assert_equal [2, 1], m.swap_prime([1, 2])
    # the suite's `Maybe` (test_helper) is the current registration; Prelude values use it
    assert_equal Just.new(3), m.safeDiv(7, 2)
    assert_equal Nothing, m.safeDiv(1, 0)
    assert_equal 3, m.unwrap(m.safeDiv(7, 2))
    assert_equal(-1, m.unwrap(m.safeDiv(1, 0)))
    assert_equal "b", m.lookupOr(2, [[1, "a"], [2, "b"]])
    assert_equal "?", m.lookupOr(5, [])
    assert_equal [[1, "a"], [2, "b"], [3, "c"]], m.pairs
  end

  def test_mutual_recursion_error_and_values
    m = hs(<<~HS)
      isEven :: Int -> Bool
      isEven 0 = True
      isEven n = isOdd (n - 1)

      isOdd :: Int -> Bool
      isOdd 0 = False
      isOdd n = isEven (n - 1)

      headOr :: [Int] -> Int
      headOr [] = error "empty!"
      headOr (x:_) = x

      answer :: Int
      answer = 6 * 7
    HS
    assert_equal [true, false], [m.isEven(10), m.isOdd(10)]
    err = assert_raises(HaskellMatch::Prelude::HaskellError) { m.headOr([]) }
    assert_equal "empty!", err.message
    assert_equal 4, m.headOr([4])
    assert_equal 42, m.answer
  end

  def test_deriving_in_haskell_declarations
    m = hs(<<~HS)
      data Color = Red | Green | Blue deriving (Eq, Ord, Enum, Bounded, Show)

      next :: Color -> Color
      next Blue = Red
      next c = succ c

      warmer :: Color -> Color -> Bool
      warmer a b = a < b

      all :: [Color]
      all = [Red ..]

      odds :: [Color]
      odds = [Red, Blue ..]

      span' :: Color -> Color -> [Color]
      span' a b = [a .. b]

      idx :: Color -> Int
      idx = fromEnum

      letters :: String
      letters = ['a' .. 'e']
    HS
    assert_equal [m::Green, m::Red], [m.next(m::Red), m.next(m::Blue)]
    assert_equal true, m.warmer(m::Red, m::Blue)
    assert_equal [m::Red, m::Green, m::Blue], m.all
    assert_equal [m::Red, m::Blue], m.odds
    assert_equal [m::Green, m::Blue], m.span_prime(m::Green, m::Blue)
    assert_equal [], m.span_prime(m::Blue, m::Green)
    assert_equal 2, m.idx(m::Blue)
    assert_equal "abcde", m.letters
    assert_equal [m::Red, m::Blue], [m::Color.min_bound, m::Color.max_bound]
    assert_raises(HaskellMatch::DataDeclarationError) { hs("data T = A Int deriving Enum") }
  end

  # ------------------------------------------------------------ laziness

  def test_lists_built_through_cons_are_lazy_like_haskells
    m = hs(<<~HS)
      myMap :: (a -> b) -> [a] -> [b]
      myMap f [] = []
      myMap f (x:xs) = f x : myMap f xs

      powers :: [Int]
      powers = iterate (*2) 1

      nats :: [Int]
      nats = [0..]

      primes :: [Int]
      primes = sieve [2..]
        where
          sieve [] = []
          sieve (p:xs) = p : sieve [x | x <- xs, x `mod` p /= 0]

      fibs :: [Int]
      fibs = 0 : 1 : zipWith (+) fibs (tail fibs)

      squares :: Int -> [Int]
      squares n = [x * x | x <- [1..n], odd x]

      countdown :: Int -> [Int]
      countdown n = take n (iterate (subtract 1) n)

      evens :: [Int]
      evens = filter even [1..20]
    HS
    mapped = m.myMap(->(x) { x * 3 }, [1, 2, 3])
    assert_kind_of HaskellMatch::LazyList, mapped
    assert_equal [3, 6, 9], mapped.to_a
    assert mapped == [3, 6, 9] # LazyList#== compares elements
    assert_equal [1, 2, 4, 8, 16, 32], m.powers.take(6)
    assert_equal [0, 1, 2, 3, 4], m.nats.take(5)
    assert_equal [2, 3, 5, 7, 11, 13, 17, 19], m.primes.take(8)
    assert_equal [0, 1, 1, 2, 3, 5, 8, 13, 21, 34], m.fibs.take(10)
    assert_equal [1, 9, 25, 49], m.squares(7)
    assert_equal [3, 2, 1], m.countdown(3)
    assert_equal [2, 4, 6, 8, 10, 12, 14, 16, 18, 20], m.evens
  end

  def test_lazy_cons_tails_are_evaluated_once_and_on_demand
    calls = 0
    cell = HaskellMatch::LazyList.lazy_cons(1) { calls += 1; [2, 3] }
    assert_equal 0, calls
    assert_equal 1, cell.head
    assert_equal 0, calls
    assert_equal [1, 2, 3], cell.to_a
    assert_equal [1, 2, 3], cell.to_a
    assert_equal 1, calls
    assert cell == [1, 2, 3]
    refute cell == [1, 2]
    refute cell == [1, 2, 3, 4]
    assert HaskellMatch::LazyList.lazy_cons("a") { "bc" } == "abc".chars
  end

  # ------------------------------------------------------------ interop

  def test_haskell_calls_ruby_methods_of_the_host_module
    host = Module.new do
      def self.shout(s)
        s.upcase
      end

      def self.twice(x)
        x * 2
      end
    end
    hs(<<~HS, into: host)
      greet :: String -> String
      greet name = shout ("hello " ++ name)

      quad :: Int -> Int
      quad = twice . twice
    HS
    assert_equal "HELLO BOB", host.greet("bob")
    assert_equal 12, host.quad(3)
  end

  def test_extend_haskell_inside_a_module_body
    mod = Module.new do
      extend HaskellMatch::Haskell
      haskell <<~HS
        data Tree = Leaf | Node Tree Int Tree

        insert :: Int -> Tree -> Tree
        insert x Leaf = Node Leaf x Leaf
        insert x t@(Node l v r)
          | x < v = Node (insert x l) v r
          | x > v = Node l v (insert x r)
          | otherwise = t

        toList :: Tree -> [Int]
        toList Leaf = []
        toList (Node l v r) = toList l ++ [v] ++ toList r

        fromList :: [Int] -> Tree
        fromList = foldr insert Leaf
      HS
    end
    assert_equal [1, 3, 5, 8], mod.toList(mod.fromList([5, 3, 8, 1, 3]))
    assert_equal [1, 3, 5, 8], mod.to_list(mod.from_list([5, 3, 8, 1]))
    # the compiled Function objects work like any other HaskellMatch function
    insert = mod.haskell_function(:insert)
    assert_equal 2, insert.arity
    assert_equal mod::Node.new(mod::Leaf, 1, mod::Leaf), insert.(1, mod::Leaf)
    assert_includes HaskellMatch::Haskell.generated_ruby(mod), 'HaskellMatch.fn("insert"'
  end

  def test_haskell_functions_mix_with_ruby_functions
    m = hs(<<~HS)
      data Op = Add | Mul

      apply :: Op -> Int -> Int -> Int
      apply Add a b = a + b
      apply Mul a b = a * b
    HS
    fold = fn(:fold) do
      on("_", "acc", "[]") { |acc| acc }
      on("op", "acc", "(x:xs)") { |op, acc, x, xs| fold.(op, m.apply(op, acc, x), xs) }
    end
    assert_equal 10, fold.(m::Add, 0, [1, 2, 3, 4])
    assert_equal 24, fold.(m::Mul, 1, [1, 2, 3, 4])
    # a module's types live in its own type scope: Ruby patterns on them go
    # through the module's `fn`/`case_of`/`pattern` (same options as HaskellMatch's)
    name = m.fn(:name) { on("Add") { "add" }; on("Mul") { "mul" } }
    assert_equal %w[add mul], [m::Add, m::Mul].map { |o| name.(o) }
    assert_equal "mul", m.case_of(m::Mul) { on("Add") { "add" }; on("Mul") { "mul" } }
    assert_equal({}, m.pattern("Add").match(m::Add))
    assert_raises(HaskellMatch::UnknownConstructorError) { fn(:global) { on("Add") { 1 } } }
    assert_raises(HaskellMatch::NonExhaustiveError) { m.fn(:partial) { on("Add") { 1 } } }
  end

  def test_each_module_has_its_own_type_scope
    a = hs("data Shape = Circle Double | Square Double\narea :: Shape -> Double\narea (Circle r) = 3 * r * r\narea (Square s) = s * s")
    b = hs("data Shape = Circle Double | Rect Double Double\narea :: Shape -> Double\narea (Circle r) = 3 * r * r\narea (Rect w h) = w * h")
    assert_equal 4.0, a.area(a::Square.new(2.0))
    assert_equal 6.0, b.area(b::Rect.new(2.0, 3.0))
    refute_equal a::Circle, b::Circle
    # the suite's global `Shape` (test_helper) is untouched
    assert_equal %w[Circle Rect Tri], HaskellMatch::Native.constructors("Shape")
    assert_equal %w[Circle Square], HaskellMatch::Native.constructors("Shape", a.haskell_scope)
    assert_equal 12.0, fn(:area) { on("Circle r") { |r| 3 * r * r }; on("_") { 0 } }.(Circle.new(2.0))
    # a module's data helper adds a type to the module's scope
    a.data "Marker = Marker"
    assert_equal "unit", a.fn(:u) { on("Marker") { "unit" } }.(a::Marker::Marker)
    assert_nil HaskellMatch::Native.constructors("Marker")
  end

  # ------------------------------------------------------------ files

  def test_load_compiles_a_file_into_a_fresh_module
    g = HaskellMatch.load(File.join(EXAMPLES, "geometry.hs"))
    shapes = [g::Circle.new(1.0), g::Rect.new(3.0, 3.0), g::Rect.new(2.0, 8.0), g::Triangle.new(3.0, 4.0, 5.0)]
    assert_in_delta Math::PI, g.area(shapes[0])
    assert_equal [9.0, 16.0, 6.0], shapes.drop(1).map { |s| g.area(s) }
    assert_equal ["small circle", "small square", "medium rectangle", "small triangle"], shapes.map { |s| g.describe(s) }
    assert_in_delta 34.14159, g.totalArea(shapes), 1e-4
    assert_equal Just.new(shapes[2]), g.largest(shapes)
    assert_equal Nothing, g.largest([])

    l = HaskellMatch.load(File.join(EXAMPLES, "lists.hs"))
    assert_equal [2, 3, 5, 7, 11, 13, 17, 19, 23, 29], l.primes.take(10)
    assert_equal [0, 1, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89], l.fibs.take(12)
    assert_equal [6, 3, 10, 5, 16, 8, 4, 2, 1], l.collatz(6).to_a
    assert_equal 500_000_500_000, l.sumTo(1_000_000)
    assert_equal [1, 1, 2, 3, 4, 5, 6, 9], l.quicksort([3, 1, 4, 1, 5, 9, 2, 6])
  end

  def test_require_finds_hs_files_on_the_load_path_and_names_the_constant
    $LOAD_PATH.unshift(EXAMPLES)
    g = HaskellMatch.require("geometry")
    assert_same g, Geometry
    assert_same g, HaskellMatch.require("geometry") # compiled once
    assert_equal 2.0, g.area(g::Rect.new(1.0, 2.0))
    err = assert_raises(LoadError) { HaskellMatch.require("no_such_module") }
    assert_includes err.message, "no_such_module.hs"
  ensure
    $LOAD_PATH.delete(EXAMPLES)
  end

  def test_require_uses_the_module_header_or_the_file_name
    Dir.mktmpdir do |dir|
      File.write(File.join(dir, "Util.hs"), "module My.Util where\nid' :: a -> a\nid' x = x\n")
      File.write(File.join(dir, "string_tools.hs"), "yell :: String -> String\nyell s = s ++ \"!\"\n")
      ns = Module.new
      m = HaskellMatch.require(File.join(dir, "Util.hs"), under: ns)
      assert_same m, ns::My::Util
      assert_equal 1, m.id_prime(1)
      m2 = HaskellMatch.require(File.join(dir, "string_tools"), under: ns)
      assert_same m2, ns::StringTools
      assert_equal "hi!", m2.yell("hi")
    end
  end

  # ------------------------------------------------------------ errors

  def test_syntax_errors_point_into_the_ruby_file
    err = assert_raises(HaskellMatch::HaskellSyntaxError) do
      hs("f x =\n  | x")
    end
    assert_match(/#{Regexp.escape(__FILE__)}:\d+:5: unexpected pipe in expression/, err.message)
    assert_kind_of HaskellMatch::CompileError, err
  end

  def test_exhaustiveness_and_redundancy_are_enforced_like_everywhere_else
    err = assert_raises(HaskellMatch::NonExhaustiveError) { hs("f :: Int -> Int\nf 0 = 1") }
    assert_match(/#{Regexp.escape(__FILE__)}:\d+: Pattern match\(es\) are non-exhaustive/, err.message)
    assert_includes err.message, "In an equation for 'f'"
    assert_nil err.cause

    err = assert_raises(HaskellMatch::RedundantClauseError) { hs("f :: Int -> Int\nf _ = 1\nf 0 = 2") }
    assert_includes err.message, "Pattern match is redundant"
    assert_match(/f 0 = \.\.\. \(#{Regexp.escape(__FILE__)}:\d+\)/, err.message)

    # where-bound helpers are checked too, and reported by name
    err = assert_raises(HaskellMatch::NonExhaustiveError) do
      hs("f xs = go xs\n  where go (x:_) = x")
    end
    assert_includes err.message, "In an equation for 'go'"

    out = capture_warnings { hs("f :: Int -> Int\nf 0 = 1", exhaustive: :warn) }
    assert_includes out, "non-exhaustive"
  end

  def test_out_of_scope_features_fail_with_clear_messages
    assert_raises_message(HaskellMatch::HaskellSyntaxError, "class declarations are not supported") do
      hs("class Foo a where\n  foo :: a -> Int")
    end
    assert_raises_message(HaskellMatch::HaskellSyntaxError, "instance declarations are not supported") do
      hs("instance Show Foo where\n  show _ = \"x\"")
    end
    assert_raises_message(HaskellMatch::HaskellSyntaxError, "'do' blocks are not supported") do
      hs("main = do\n  putStrLn \"hi\"")
    end
    assert_raises_message(HaskellMatch::UnknownConstructorError, "data constructor 'Frob'") do
      hs("f x = Frob x")
    end
    assert_raises_message(HaskellMatch::DuplicateVariableError, "conflicting definitions for 'x'") do
      hs("f x x = x")
    end
  end

  def test_unknown_names_are_assumed_to_be_ruby_methods_of_the_host
    m = hs("f x = frob x")
    assert_raises(NoMethodError) { m.f(1) }
    m.define_singleton_method(:frob) { |x| x + 1 }
    assert_equal 2, m.f(1)
  end
end
