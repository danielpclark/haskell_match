# frozen_string_literal: true

module HaskellMatch
  # Patterns written in place inside a definition block, without quotes.
  # Each expression builds the pattern tree directly in Ruby and is rendered
  # to Haskell pattern syntax, then compiled exactly like a quoted pattern, so
  # matching, exhaustiveness checking and error messages are identical.
  #
  #   on([])                { 0 }
  #   on([x, *xs])          { |x, xs| ... }       # (x:xs); also cons(x, xs)
  #   on(Just(Just(_)))     { ... }               # or Just[Just[_]]
  #   on(Person(name: n))   { |n| n }             # Person { name = n }
  #   on(Person(name: n, **_)) { |n| n }          # Person { name = n, .. }
  #   on(tuple(a, b))       { |a, b| a + b }      # (a, b)
  #   on(as(all, [x, *_]))  { |all, x| ... }      # all@(x:_)
  #   on(lazy(tuple(a, b))) { ... }               # ~(a, b)
  #   on(0) ... on(1.5) ... on(str("abc")) ... on(char("c")) ... on(:sym) ... on(true)
  #
  # A String given directly to `on` is quoted Haskell pattern syntax; inside a
  # pattern (`Just("abc")`) a Ruby String is a string literal, and `str("abc")`
  # makes one at the top level.
  # Bare lower-case names are variables (use `var(:name)` when the name is a
  # method of the enclosing object or a local variable), `_` is the wildcard.
  # Quoted and in-place patterns may be mixed freely.
  module PatternAST
    class Node
      # `*node` inside an array literal marks the rest of a list.
      def to_a
        [Rest.new(self)]
      end

      # `**_` inside a record pattern stands for `..`.
      def to_hash
        { REST_KEY => true }
      end

      def inspect
        "#<HaskellMatch pattern #{to_haskell}>"
      end
      alias to_s inspect
    end

    REST_KEY = :"haskell_match.rest"

    class Var < Node
      attr_reader :name

      def initialize(name)
        @name = name.to_s
        unless @name.match?(/\A[a-z_][A-Za-z0-9_']*\z/)
          raise DefinitionError, "'#{@name}' is not a valid pattern variable name (lower-case identifier)"
        end
        freeze
      end

      def to_haskell(_atomic = false)
        name
      end
    end

    class Wild < Node
      def to_haskell(_atomic = false)
        "_"
      end
    end

    WILD = Wild.new.freeze

    # `*xs` inside an array literal: the rest of the list.
    class Rest < Node
      attr_reader :pattern

      def initialize(pattern)
        @pattern = pattern
        freeze
      end

      def to_haskell(_atomic = false)
        raise DefinitionError, "a splat (*rest) may only appear last in a list pattern"
      end
    end

    class ConApp < Node
      attr_reader :name, :args

      def initialize(name, args)
        @name = name.to_s
        @args = args.freeze
        freeze
      end

      def to_haskell(atomic = false)
        hs = HaskellMatch.symbolic_constructor(name) || name
        return hs if args.empty?

        s = if hs.start_with?(":") && args.size == 2
              "#{PatternAST.render(args[0], true)} #{hs} #{PatternAST.render(args[1], true)}"
            else
              "#{hs.start_with?(':') ? "(#{hs})" : hs} #{args.map { |a| PatternAST.render(a, true) }.join(' ')}"
            end
        atomic ? "(#{s})" : s
      end
    end

    class Record < Node
      attr_reader :name, :fields, :rest

      def initialize(name, fields, rest)
        @name = name.to_s
        @fields = fields.freeze
        @rest = rest
        freeze
      end

      def to_haskell(_atomic = false)
        parts = fields.map { |f, p| "#{f} = #{PatternAST.render(p)}" }
        parts << ".." if rest
        "#{name} {#{parts.join(', ')}}"
      end
    end

    class Tuple < Node
      attr_reader :items

      def initialize(items)
        @items = items.freeze
        freeze
      end

      def to_haskell(_atomic = false)
        "(#{items.map { |i| PatternAST.render(i) }.join(', ')})"
      end
    end

    class Cons < Node
      attr_reader :head, :tail

      def initialize(head, tail)
        @head = head
        @tail = tail
        freeze
      end

      def to_haskell(_atomic = false)
        "(#{PatternAST.render(head, true)}:#{PatternAST.render(tail, true)})"
      end
    end

    class Prefixed < Node
      attr_reader :prefix, :pattern

      def initialize(prefix, pattern)
        @prefix = prefix
        @pattern = pattern
        freeze
      end

      def to_haskell(_atomic = false)
        "#{prefix}#{PatternAST.render(pattern, true)}"
      end
    end

    class As < Node
      attr_reader :name, :pattern

      def initialize(name, pattern)
        @name = name.is_a?(Var) ? name : Var.new(name)
        @pattern = pattern
        freeze
      end

      def to_haskell(_atomic = false)
        "#{name.to_haskell}@#{PatternAST.render(pattern, true)}"
      end
    end

    class Str < Node
      attr_reader :string

      def initialize(string)
        @string = string.to_s
        freeze
      end

      def to_haskell(_atomic = false)
        "\"#{PatternAST.escape(string, '"')}\""
      end
    end

    class Char < Node
      attr_reader :char

      def initialize(str)
        s = str.to_s
        raise DefinitionError, "char(#{str.inspect}) needs exactly one character" unless s.length == 1

        @char = s
        freeze
      end

      def to_haskell(_atomic = false)
        "'#{PatternAST.escape(char, "'")}'"
      end
    end

    module_function

    # Render any supported Ruby object as Haskell pattern text.
    def render(obj, atomic = false)
      case obj
      when Node then obj.to_haskell(atomic)
      when String then "\"#{escape(obj, '"')}\""
      when Symbol then symbol(obj)
      when Integer then obj.negative? && atomic ? "(#{obj})" : obj.to_s
      when Float then float(obj, atomic)
      when true then "True"
      when false then "False"
      when Array then list(obj)
      when Hash then hash_pattern(obj)
      when Constructor then constructor(obj, atomic)
      when Class
        if obj < Data && obj.include?(Constructor)
          obj.constructor_name # the compiler reports the arity error
        else
          unsupported(obj)
        end
      when nil
        raise DefinitionError, "nil is not a Haskell pattern (use _ for a wildcard)"
      else
        unsupported(obj)
      end
    end

    def unsupported(obj)
      raise DefinitionError, "#{obj.inspect} (#{obj.class}) cannot be used as a pattern"
    end

    def list(items)
      return "[]" if items.empty?

      if items.last.is_a?(Rest)
        *init, rest = items
        return render(rest.pattern) if init.empty?

        inner = init.map { |i| render(i, true) } + [render(rest.pattern, true)]
        return "(#{inner.join(':')})"
      end
      if items.any? { |i| i.is_a?(Rest) }
        raise DefinitionError, "a splat (*rest) may only appear last in a list pattern"
      end

      "[#{items.map { |i| render(i) }.join(', ')}]"
    end

    # `{name: n, "k" => v}` -> `{name = n, "k" = v}`: a Hash having those keys
    # (Symbol or String), whose values match the sub-patterns; other keys are
    # ignored.  `**_` (the record-pattern rest marker) is accepted and means
    # nothing extra, since Hash patterns are always open.
    def hash_pattern(hash)
      parts = hash.filter_map do |k, v|
        next if k == REST_KEY

        key = case k
              when Symbol then k.to_s.match?(/\A[a-z_][A-Za-z0-9_']*\z/) ? k.to_s : ":#{k.to_s.inspect}"
              when String then k.inspect
              else raise DefinitionError, "Hash pattern keys must be Symbols or Strings (got #{k.inspect})"
              end
        "#{key} = #{render(v)}"
      end
      "{#{parts.join(', ')}}"
    end

    def constructor(value, atomic)
      name = value.class.constructor_name
      fields = value.fields
      return name if fields.empty?

      s = if name.to_s.start_with?(":") && fields.size == 2
            "#{render(fields[0], true)} #{name} #{render(fields[1], true)}"
          else
            "#{name} #{fields.map { |f| render(f, true) }.join(' ')}"
          end
      atomic ? "(#{s})" : s
    end

    def symbol(sym)
      s = sym.to_s
      if s.match?(/\A[A-Za-z_][A-Za-z0-9_]*[?!]?\z/)
        ":#{s}"
      else
        ":\"#{escape(s, '"')}\""
      end
    end

    def float(f, atomic)
      raise DefinitionError, "#{f} cannot be used as a pattern" unless f.finite?

      s = f.to_s
      f.negative? && atomic ? "(#{s})" : s
    end

    # Escape for the pattern lexer's string/char literals.
    def escape(str, quote)
      str.each_char.map do |c|
        case c
        when "\\" then "\\\\"
        when quote then "\\#{quote}"
        when "\n" then "\\n"
        when "\t" then "\\t"
        when "\r" then "\\r"
        when "\0" then "\\0"
        else
          c.ord < 0x20 ? format("\\x%02x", c.ord) : c
        end
      end.join
    end

    # Methods mixed into clause builders so patterns can be written in place.
    module BuilderMethods
      def _
        WILD
      end

      def var(name)
        Var.new(name)
      end

      def cons(head, tail)
        Cons.new(head, tail)
      end

      def tuple(*items)
        Tuple.new(items)
      end

      def unit
        Tuple.new([])
      end

      def as(name, pattern)
        As.new(name, pattern)
      end

      def lazy(pattern)
        Prefixed.new("~", pattern)
      end

      def bang(pattern)
        Prefixed.new("!", pattern)
      end

      def char(str)
        Char.new(str)
      end

      # A string literal pattern.  Needed only as a whole argument of `on`,
      # where a bare String is read as quoted Haskell pattern syntax; inside
      # a pattern (`Just("abc")`, `[str, *rest]`) Ruby Strings are literals.
      def str(string)
        Str.new(string)
      end

      # `Just(x)`, `Person(name: n, age: _)`, `Node(l, v, r)`; a bare
      # lower-case name is a pattern variable.
      def method_missing(name, *args, **kwargs, &blk)
        s = name.to_s
        if s.match?(/\A[A-Z]/) && blk.nil?
          if kwargs.empty?
            ConApp.new(s, args)
          elsif args.empty?
            rest = kwargs.delete(REST_KEY) ? true : false
            Record.new(s, kwargs.map { |k, v| [k.to_s, v] }, rest)
          else
            raise DefinitionError, "#{s}(...) takes either positional or named fields, not both"
          end
        elsif args.empty? && kwargs.empty? && blk.nil? && s.match?(/\A[a-z_][A-Za-z0-9_]*\z/) &&
              !pattern_owner_responds?(name)
          Var.new(s)
        else
          super
        end
      end

      def respond_to_missing?(name, include_private = false)
        name.to_s.match?(/\A[A-Za-z_]/) || super
      end

      # Hook: builders with an owner delegate names the owner knows.
      def pattern_owner_responds?(_name)
        false
      end
    end
  end
end
