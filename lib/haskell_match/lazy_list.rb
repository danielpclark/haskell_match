# frozen_string_literal: true

module HaskellMatch
  # A lazy, memoised cons list: Haskell's list, including infinite ones.
  #
  #   naturals = HaskellMatch.lazy(1..)                 # or any Enumerable / Enumerator
  #   take = HaskellMatch.fn(:take) do
  #     on("0", "_")      { [] }
  #     on("_", "[]")     { [] }
  #     on("n", "(x:xs)") { |n, x, xs| [x] + take.(n - 1, xs) }
  #   end
  #   take.(5, naturals)                                # => [1, 2, 3, 4, 5]
  #
  # List patterns (`[]`, `(x:xs)`, `[a, b]`) match a LazyList directly; `xs`
  # is bound to the rest of the list, still lazy. An Enumerator (including
  # `Enumerator::Lazy`) used as a value is wrapped automatically; each wrap
  # iterates it from the start, so the same stream matches the same way every
  # time. Elements are computed once and shared, as in Haskell.
  class LazyList
    include Enumerable

    # Build a lazy list over any Enumerable (an Enumerator is iterated from
    # its start; the caller's enumerator is left untouched).
    def self.from(source)
      return source if source.is_a?(LazyList)
      raise TypeError, "#{source.inspect} is not Enumerable" unless source.respond_to?(:each)

      new(source.to_enum(:each))
    end

    # The empty lazy list.
    def self.empty
      from([])
    end

    # An infinite list `x, f(x), f(f(x)), ...` (Haskell's `iterate`).
    def self.iterate(seed, &step)
      from(Enumerator.produce(seed, &step))
    end

    # A lazy list from a block that yields elements (`yielder << x`).
    def self.generate(&block)
      from(Enumerator.new(&block))
    end

    # An infinite repetition of `value` (Haskell's `repeat`).
    def self.repeat(value)
      from(Enumerator.produce(value) { value })
    end

    # A finite or infinite list from `first` upwards (`[n..]`, `[n..m]`).
    def self.range(first, last = nil)
      from(last ? (first..last) : (first..))
    end

    # @api private: `enum` is an external enumerator shared along the spine.
    def initialize(enum)
      @enum = enum
      @state = :unforced
    end

    # Evaluate this cell: nil for the empty list, `[head, tail]` for a cons.
    # Called by the native matcher; memoised, so each element is produced
    # once.
    def force
      if @state == :unforced
        begin
          @head = @enum.next
          @tail = LazyList.new(@enum)
          @state = :cons
        rescue StopIteration
          @state = :nil
        end
      end
      @state == :cons ? [@head, @tail] : nil
    end

    def empty?
      force.nil?
    end

    def head
      pair = force or raise MatchError, "head of an empty list"
      pair[0]
    end

    def tail
      pair = force or raise MatchError, "tail of an empty list"
      pair[1]
    end

    # Iterate the elements (does not terminate for an infinite list).
    def each
      return to_enum(:each) unless block_given?

      cell = self
      while (pair = cell.force)
        yield pair[0]
        cell = pair[1]
      end
      self
    end

    # The first `n` elements, as a (strict) Array.
    def take(n)
      out = []
      cell = self
      while out.size < n && (pair = cell.force)
        out << pair[0]
        cell = pair[1]
      end
      out
    end

    # Elements already evaluated, without forcing more.
    def forced_prefix
      out = []
      cell = self
      while cell.instance_variable_get(:@state) == :cons
        out << cell.instance_variable_get(:@head)
        cell = cell.instance_variable_get(:@tail)
      end
      [out, cell.instance_variable_get(:@state) == :nil]
    end

    def inspect
      prefix, complete = forced_prefix
      items = prefix.map(&:inspect)
      items << "..." unless complete
      "LazyList[#{items.join(', ')}]"
    end
    alias to_s inspect
  end

  class << self
    # Build a {LazyList} from any Enumerable.
    def lazy(source)
      LazyList.from(source)
    end
  end
end
