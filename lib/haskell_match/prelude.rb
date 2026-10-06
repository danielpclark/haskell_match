# frozen_string_literal: true

module HaskellMatch
  # The functions Haskell code compiled by haskell_match can call without
  # defining them.  Each is a Ruby module function of fixed arity; `curried`
  # gives the curried Proc used when a function is passed as a value.
  #
  # Lists are Ruby Arrays, Strings (lists of characters) or {LazyList}s;
  # functions that build lists stay lazy when their input is lazy.
  module Prelude
    # Raised by `error` and `undefined`.
    class HaskellError < Error; end

    ARITY = {} # name => arity
    @curried = {}

    class << self
      def defn(name, arity, &impl)
        ARITY[name] = arity
        define_singleton_method(name, &impl)
      end

      def arity(name)
        ARITY[name]
      end

      def known?(name)
        ARITY.key?(name)
      end

      # The function as a curried Proc (memoised).
      def curried(name)
        @curried[name] ||= begin
          m = method(name)
          case ARITY.fetch(name)
          when 0 then m.to_proc
          when 1 then m.to_proc
          else m.to_proc.curry
          end
        end
      end

      # Curry any callable of known arity.
      def curry(callable, arity)
        return callable if arity <= 1

        callable.to_proc.curry(arity)
      end

      # ---- list plumbing ---------------------------------------------------

      def lazy?(xs)
        xs.is_a?(LazyList) || xs.is_a?(Enumerator)
      end

      # An Enumerator::Lazy over any list value.
      def lazy_enum(xs)
        case xs
        when Array then xs.lazy
        when String then xs.each_char.lazy
        when LazyList then xs.each.lazy
        when Enumerator then xs.lazy
        else raise TypeError, "#{xs.inspect} is not a list"
        end
      end

      # Materialise an Enumerable into the kind of list `like` was.
      def relist(enum, like)
        return LazyList.from(enum) if lazy?(like)

        out = enum.to_a
        out = out.join if like.is_a?(String) && out.all? { |c| c.is_a?(String) && c.length == 1 }
        out
      end

      # Prelude values go through the registry so they stay compatible with a
      # program that redeclares `Maybe` or `Ordering` itself.
      def just(x)
        HaskellMatch.constructor("Just").new(x)
      end

      def nothing
        HaskellMatch.constructor("Nothing")
      end

      def just?(m)
        m.is_a?(Constructor) && m.class.constructor_name == "Just"
      end

      # ---- operators used by generated code --------------------------------

      def cons(x, xs)
        case xs
        when LazyList, Enumerator then LazyList.cons(x, xs)
        when String then x.is_a?(String) ? x + xs : [x] + xs.chars
        when Array then [x] + xs
        else raise TypeError, "cannot cons onto #{xs.inspect}"
        end
      end

      def append(xs, ys)
        if lazy?(xs) || lazy?(ys)
          return LazyList.from(Enumerator.new { |y| lazy_enum(xs).each { |e| y << e }; lazy_enum(ys).each { |e| y << e } })
        end
        return xs + ys if xs.class == ys.class

        (xs.is_a?(String) ? xs.chars : xs) + (ys.is_a?(String) ? ys.chars : ys)
      end

      def compose(f, g)
        ->(x) { f.(g.(x)) }
      end

      def fdiv(a, b)
        a.is_a?(Integer) && b.is_a?(Integer) ? a.fdiv(b) : a / b
      end

      def powf(a, b)
        a.to_f**b
      end

      def index(xs, n)
        raise HaskellError, "Prelude.!!: negative index" if n.negative?

        v = case xs
            when LazyList then xs.take(n + 1)[n]
            when Enumerator then xs.lazy.first(n + 1)[n]
            else xs[n]
            end
        raise HaskellError, "Prelude.!!: index too large" if v.nil? && !(xs.respond_to?(:size) && n < xs.size)

        v
      end

      # [from..], [from..to], [from, then ..], [from, then .. to]
      def range(from, step_to = nil, to = nil)
        step = step_to.nil? ? 1 : step_to - from
        if to.nil?
          return LazyList.from(Enumerator.produce(from) { |x| x + step })
        end
        return [] if step.zero? && from > to

        out = []
        x = from
        if step.positive?
          while x <= to
            out << x
            x += step
          end
        elsif step.negative?
          while x >= to
            out << x
            x += step
          end
        else
          raise HaskellError, "enumFromThenTo: zero step"
        end
        out
      end

      def op(name)
        @ops ||= {
          "+" => ->(a) { ->(b) { a + b } }, "-" => ->(a) { ->(b) { a - b } }, "*" => ->(a) { ->(b) { a * b } },
          "/" => ->(a) { ->(b) { fdiv(a, b) } }, "^" => ->(a) { ->(b) { a**b } }, "**" => ->(a) { ->(b) { powf(a, b) } },
          "==" => ->(a) { ->(b) { a == b } }, "/=" => ->(a) { ->(b) { a != b } },
          "<" => ->(a) { ->(b) { a < b } }, "<=" => ->(a) { ->(b) { a <= b } },
          ">" => ->(a) { ->(b) { a > b } }, ">=" => ->(a) { ->(b) { a >= b } },
          "&&" => ->(a) { ->(b) { a && b } }, "||" => ->(a) { ->(b) { a || b } },
          "++" => ->(a) { ->(b) { append(a, b) } }, ":" => ->(a) { ->(b) { cons(a, b) } },
          "." => ->(f) { ->(g) { compose(f, g) } }, "$" => ->(f) { ->(x) { f.(x) } },
          "!!" => ->(a) { ->(b) { index(a, b) } }
        }.freeze
        @ops.fetch(name) do
          raise HaskellError, "operator #{name} is not supported" unless known?(name.to_sym)

          curried(name.to_sym)
        end
      end

      # ---- list comprehensions ----------------------------------------------

      # Each comprehension evaluates its generators through `gen`, which
      # records whether any source was lazy; `finish` then returns a LazyList
      # or an Array accordingly.
      def comp_begin
        (@comp_lazy ||= []).push(false)
      end

      def gen(xs)
        @comp_lazy[-1] = true if lazy?(xs)
        lazy_enum(xs)
      end

      def comp_finish(enum)
        lazy = @comp_lazy.pop
        lazy ? LazyList.from(enum) : enum.to_a
      end
    end

    # ---- the Prelude proper ----------------------------------------------------

    defn(:id, 1) { |x| x }
    defn(:const, 2) { |x, _y| x }
    defn(:flip, 3) { |f, x, y| f.(y).(x) }
    defn(:seq, 2) { |_a, b| b }
    defn(:fst, 1) { |p| p[0] }
    defn(:snd, 1) { |p| p[1] }
    defn(:curry, 3) { |f, a, b| f.([a, b]) }
    defn(:uncurry, 2) { |f, p| f.(p[0]).(p[1]) }
    defn(:not, 1) { |b| !b }
    defn(:otherwise, 0) { true }
    defn(:error, 1) { |msg| raise HaskellError, msg.to_s }
    defn(:undefined, 0) { raise HaskellError, "Prelude.undefined" }
    defn(:show, 1) { |x| x.is_a?(String) ? x.inspect : x.inspect }
    defn(:putStrLn, 1) { |s| puts s; [] }
    defn(:putStr, 1) { |s| print s; [] }
    defn(:print, 1) { |x| puts show(x); [] }

    # numbers
    defn(:even, 1) { |n| n.even? }
    defn(:odd, 1) { |n| n.odd? }
    defn(:negate, 1) { |n| -n }
    defn(:abs, 1) { |n| n.abs }
    defn(:signum, 1) { |n| n <=> 0 }
    defn(:succ, 1) { |x| x.is_a?(String) ? (x.ord + 1).chr : x + 1 }
    defn(:pred, 1) { |x| x.is_a?(String) ? (x.ord - 1).chr : x - 1 }
    defn(:min, 2) { |a, b| a <= b ? a : b }
    defn(:max, 2) { |a, b| a >= b ? a : b }
    defn(:subtract, 2) { |a, b| b - a }
    defn(:div, 2) { |a, b| a.div(b) }
    defn(:mod, 2) { |a, b| a % b }
    defn(:quot, 2) { |a, b| (a.to_f / b).truncate }
    defn(:rem, 2) { |a, b| a - b * (a.to_f / b).truncate }
    defn(:divMod, 2) { |a, b| a.divmod(b) }
    defn(:quotRem, 2) { |a, b| q = (a.to_f / b).truncate; [q, a - b * q] }
    defn(:gcd, 2) { |a, b| a.gcd(b) }
    defn(:lcm, 2) { |a, b| a.lcm(b) }
    defn(:fromIntegral, 1) { |n| n }
    defn(:fromInteger, 1) { |n| n }
    defn(:toInteger, 1) { |n| n.to_i }
    defn(:realToFrac, 1) { |n| n.to_f }
    defn(:truncate, 1) { |x| x.truncate }
    defn(:round, 1) { |x| x.round(half: :even) }
    defn(:floor, 1) { |x| x.floor }
    defn(:ceiling, 1) { |x| x.ceil }
    defn(:sqrt, 1) { |x| Math.sqrt(x) }
    defn(:exp, 1) { |x| Math.exp(x) }
    defn(:log, 1) { |x| Math.log(x) }
    defn(:sin, 1) { |x| Math.sin(x) }
    defn(:cos, 1) { |x| Math.cos(x) }
    defn(:tan, 1) { |x| Math.tan(x) }
    defn(:pi, 0) { Math::PI }
    defn(:compare, 2) do |a, b|
      case a <=> b
      when -1 then HaskellMatch.constructor("LT")
      when 0 then HaskellMatch.constructor("EQ")
      when 1 then HaskellMatch.constructor("GT")
      else raise HaskellError, "Prelude.compare: #{a.inspect} and #{b.inspect} are not comparable"
      end
    end

    # characters
    defn(:ord, 1) { |c| c.ord }
    defn(:chr, 1) { |n| n.chr(Encoding::UTF_8) }
    defn(:toUpper, 1) { |c| c.upcase }
    defn(:toLower, 1) { |c| c.downcase }
    defn(:isDigit, 1) { |c| c.match?(/\A\d\z/) }
    defn(:isAlpha, 1) { |c| c.match?(/\A\p{Alpha}\z/) }
    defn(:isSpace, 1) { |c| c.match?(/\A\s\z/) }
    defn(:isUpper, 1) { |c| c.match?(/\A\p{Upper}\z/) }
    defn(:isLower, 1) { |c| c.match?(/\A\p{Lower}\z/) }
    defn(:digitToInt, 1) { |c| c.to_i(16) }
    defn(:intToDigit, 1) { |n| n.to_s(16) }

    # Maybe
    defn(:maybe, 3) { |d, f, m| just?(m) ? f.(m.fields[0]) : d }
    defn(:fromMaybe, 2) { |d, m| just?(m) ? m.fields[0] : d }
    defn(:isJust, 1) { |m| just?(m) }
    defn(:isNothing, 1) { |m| !just?(m) }
    defn(:listToMaybe, 1) { |xs| xs = lazy_enum(xs).first(1); xs.empty? ? nothing : just(xs[0]) }
    defn(:maybeToList, 1) { |m| just?(m) ? [m.fields[0]] : [] }
    defn(:either, 3) { |f, g, e| e.class.constructor_name == "Left" ? f.(e.fields[0]) : g.(e.fields[0]) }

    # lists
    defn(:head, 1) { |xs| lazy_enum(xs).first(1).fetch(0) { raise HaskellError, "Prelude.head: empty list" } }
    defn(:tail, 1) do |xs|
      case xs
      when LazyList then xs.tail
      when Enumerator then LazyList.from(xs).tail
      else
        raise HaskellError, "Prelude.tail: empty list" if xs.empty?

        xs[1..]
      end
    end
    defn(:last, 1) do |xs|
      raise HaskellError, "Prelude.last: empty list" if xs.empty?

      xs[-1]
    end
    defn(:init, 1) do |xs|
      raise HaskellError, "Prelude.init: empty list" if xs.empty?

      xs[0...-1]
    end
    defn(:null, 1) { |xs| lazy?(xs) ? lazy_enum(xs).first(1).empty? : xs.empty? }
    defn(:length, 1) { |xs| lazy?(xs) ? lazy_enum(xs).count : xs.length }
    defn(:reverse, 1) { |xs| lazy?(xs) ? lazy_enum(xs).to_a.reverse : xs.reverse }
    defn(:map, 2) { |f, xs| relist(lazy_enum(xs).map { |x| f.(x) }, xs) }
    defn(:filter, 2) { |p, xs| relist(lazy_enum(xs).select { |x| p.(x) }, xs) }
    defn(:concat, 1) { |xss| relist(lazy_enum(xss).flat_map { |xs| lazy_enum(xs) }, xss) }
    defn(:concatMap, 2) { |f, xs| relist(lazy_enum(xs).flat_map { |x| lazy_enum(f.(x)) }, xs) }
    defn(:foldr, 3) { |f, z, xs| lazy_enum(xs).to_a.reverse_each.inject(z) { |acc, x| f.(x).(acc) } }
    defn(:foldl, 3) { |f, z, xs| lazy_enum(xs).inject(z) { |acc, x| f.(acc).(x) } }
    defn(:"foldl'", 3) { |f, z, xs| lazy_enum(xs).inject(z) { |acc, x| f.(acc).(x) } }
    defn(:foldr1, 2) do |f, xs|
      a = lazy_enum(xs).to_a
      raise HaskellError, "Prelude.foldr1: empty list" if a.empty?

      a[0...-1].reverse_each.inject(a[-1]) { |acc, x| f.(x).(acc) }
    end
    defn(:foldl1, 2) do |f, xs|
      a = lazy_enum(xs).to_a
      raise HaskellError, "Prelude.foldl1: empty list" if a.empty?

      a[1..].inject(a[0]) { |acc, x| f.(acc).(x) }
    end
    defn(:scanl, 3) do |f, z, xs|
      acc = z
      out = [z]
      lazy_enum(xs).each { |x| acc = f.(acc).(x); out << acc }
      out
    end
    defn(:scanr, 3) do |f, z, xs|
      lazy_enum(xs).to_a.reverse_each.inject([z]) { |out, x| out.unshift(f.(x).(out[0])) }
    end
    defn(:sum, 1) { |xs| lazy_enum(xs).inject(0) { |a, b| a + b } }
    defn(:product, 1) { |xs| lazy_enum(xs).inject(1) { |a, b| a * b } }
    defn(:maximum, 1) { |xs| lazy_enum(xs).max || raise(HaskellError, "Prelude.maximum: empty list") }
    defn(:minimum, 1) { |xs| lazy_enum(xs).min || raise(HaskellError, "Prelude.minimum: empty list") }
    defn(:and, 1) { |xs| lazy_enum(xs).all? }
    defn(:or, 1) { |xs| lazy_enum(xs).any? }
    defn(:all, 2) { |p, xs| lazy_enum(xs).all? { |x| p.(x) } }
    defn(:any, 2) { |p, xs| lazy_enum(xs).any? { |x| p.(x) } }
    defn(:elem, 2) { |x, xs| lazy_enum(xs).include?(x) }
    defn(:notElem, 2) { |x, xs| !lazy_enum(xs).include?(x) }
    defn(:lookup, 2) do |k, pairs|
      found = lazy_enum(pairs).find { |p| p[0] == k }
      found ? just(found[1]) : nothing
    end
    defn(:take, 2) do |n, xs|
      case xs
      when LazyList then xs.take(n)
      when Enumerator then xs.lazy.first(n)
      else xs[0, [n, 0].max] || xs[0, 0]
      end
    end
    defn(:drop, 2) do |n, xs|
      case xs
      when LazyList, Enumerator then LazyList.from(lazy_enum(xs).drop(n))
      else xs[[n, 0].max..] || xs[0, 0]
      end
    end
    defn(:splitAt, 2) { |n, xs| [take(n, xs), drop(n, xs)] }
    defn(:takeWhile, 2) { |p, xs| relist(lazy_enum(xs).take_while { |x| p.(x) }, xs) }
    defn(:dropWhile, 2) { |p, xs| relist(lazy_enum(xs).drop_while { |x| p.(x) }, xs) }
    defn(:span, 2) { |p, xs| [takeWhile(p, xs), dropWhile(p, xs)] }
    defn(:break, 2) { |p, xs| span(->(x) { !p.(x) }, xs) }
    defn(:zipWith, 3) { |f, xs, ys| relist(zip_enum(xs, ys).map { |a, b| f.(a).(b) }, lazy_pair(xs, ys)) }
    defn(:zip3, 3) { |xs, ys, zs| zip_enum(zip_enum(xs, ys), zs).map { |(a, b), c| [a, b, c] }.to_a }
    defn(:unzip, 1) { |ps| a = lazy_enum(ps).to_a; [a.map { |p| p[0] }, a.map { |p| p[1] }] }
    defn(:replicate, 2) { |n, x| Array.new([n, 0].max, x) }
    defn(:iterate, 2) { |f, x| LazyList.iterate(x) { |v| f.(v) } }
    defn(:repeat, 1) { |x| LazyList.repeat(x) }
    defn(:cycle, 1) do |xs|
      a = lazy_enum(xs).to_a
      raise HaskellError, "Prelude.cycle: empty list" if a.empty?

      LazyList.from(a.cycle)
    end
    defn(:until, 3) do |p, f, x|
      x = f.(x) until p.(x)
      x
    end
    defn(:words, 1) { |s| s.split }
    defn(:unwords, 1) { |ws| ws.join(" ") }
    defn(:lines, 1) { |s| s.split("\n") }
    defn(:unlines, 1) { |ls| ls.map { |l| "#{l}\n" }.join }
    defn(:sort, 1) { |xs| lazy_enum(xs).to_a.sort }
    defn(:sortBy, 2) { |cmp, xs| lazy_enum(xs).to_a.sort { |a, b| cmp.(a).(b) } }
    defn(:nub, 1) { |xs| relist(lazy_enum(xs).uniq, xs) }
    defn(:partition, 2) { |p, xs| lazy_enum(xs).to_a.partition { |x| p.(x) } }
    defn(:find, 2) { |p, xs| x = lazy_enum(xs).find { |v| p.(v) }; x.nil? ? nothing : just(x) }

    class << self
      # zip stops at the shorter list; works with one infinite side
      def zip_enum(xs, ys)
        a = lazy_enum(xs)
        b = lazy_enum(ys)
        Enumerator.new do |y|
          ea = a.each
          eb = b.each
          loop { y << [ea.next, eb.next] }
        end.lazy
      end

      def lazy_pair(xs, ys)
        lazy?(xs) && lazy?(ys) ? xs : (lazy?(xs) ? ys : xs)
      end
    end
    ARITY[:zip] = 2
    defn(:zip, 2) { |xs, ys| relist(zip_enum(xs, ys), lazy_pair(xs, ys)) }
  end
  # The standard types every Haskell program can use.  They live under
  # `HaskellMatch::Prelude` (`Prelude::Maybe::Just`); a program may redeclare
  # one, which replaces the registration like any other `HaskellMatch.data`.
  HaskellMatch.data "Maybe a = Nothing | Just a", under: Prelude
  HaskellMatch.data "Either a b = Left a | Right b", under: Prelude
  HaskellMatch.data "Ordering = LT | EQ | GT", under: Prelude
end

