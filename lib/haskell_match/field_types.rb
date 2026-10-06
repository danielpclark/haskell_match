# frozen_string_literal: true

module HaskellMatch
  # A constructor argument of the wrong type (`HaskellMatch.data` with
  # `check_types: true`).
  class FieldTypeError < Error; end

  # Optional run-time checks of the field types written in a `data`
  # declaration:
  #
  #   HaskellMatch.data "Person = Person { name :: String, age :: Int }", check_types: true
  #   Person.new("Al", "3")   # raises FieldTypeError: field 'age' expects Int, got "3" (String)
  #
  # Checks follow the declared Haskell type: `Int`/`Integer` take Integers,
  # `Double`/`Float` any Numeric, `String` a String (or a list of
  # characters), `Char` a one-character String, `Bool` true/false, `[a]` any
  # list (Array, String, LazyList, Enumerator), `(a, b)` an Array of that
  # size with each element checked, `a -> b` anything callable, a declared
  # data type (`Maybe a`, `Shape`) a value of that type, any other
  # capitalised name a Ruby class or module of that name (`Hash`, `Time`,
  # `MyApp::Money`) when one exists, and a type variable anything.
  module FieldTypes
    module_function

    # Wrap `klass`'s constructor so each field is checked against `types`.
    def install(klass, types, scope)
      checkers = types.map { |t| checker(t, scope) }
      return if checkers.all?(&:nil?)

      names = klass.members
      cname = klass.constructor_name
      klass.define_method(:initialize) do |**kw|
        names.each_with_index do |n, i|
          next unless kw.key?(n)

          check = checkers[i]
          next if check.nil? || check.call(kw[n])

          raise FieldTypeError,
                "#{cname}: field '#{n}' expects #{types[i]}, got #{kw[n].inspect} (#{kw[n].class})"
        end
        super(**kw)
      end
    end

    LIST = ->(v) { v.is_a?(Array) || v.is_a?(String) || v.is_a?(LazyList) || v.is_a?(Enumerator) }

    # A predicate for Haskell type text, or nil when the type is unchecked.
    def checker(text, scope = Native::GLOBAL_SCOPE)
      t = strip_parens(text.to_s.strip)
      return nil if t.empty?
      return ->(v) { v.respond_to?(:call) } if split_top(t, "->").size > 1

      return ->(v) { v == [] } if t == "()"
      if t.start_with?("(") && t.end_with?(")")
        parts = split_top(t[1..-2], ",")
        if parts.size > 1
          subs = parts.map { |part| checker(part, scope) }
          return lambda { |v|
            v.is_a?(Array) && v.size == subs.size && subs.each_with_index.all? { |c, i| c.nil? || c.call(v[i]) }
          }
        end
      end
      return LIST if t.start_with?("[") && t.end_with?("]")
      return nil if t.match?(/\A[a-z]/) # a type variable

      head = t.split(/\s+/).first.sub(/\A!/, "")
      case head
      when "Int", "Integer", "Word", "Int8", "Int16", "Int32", "Int64", "Word8", "Word16", "Word32", "Word64", "Natural"
        ->(v) { v.is_a?(Integer) }
      when "Double", "Float", "Rational", "Num", "Real", "Fractional"
        ->(v) { v.is_a?(Numeric) }
      when "String", "Text"
        ->(v) { v.is_a?(String) || v.is_a?(Array) || v.is_a?(LazyList) }
      when "Char"
        ->(v) { v.is_a?(String) && v.length == 1 }
      when "Bool"
        ->(v) { v == true || v == false }
      when "Symbol"
        ->(v) { v.is_a?(Symbol) }
      else
        mod = HaskellMatch.type_module(head, scope)
        return ->(v) { mod === v } if mod # rubocop:disable Style/CaseEquality

        const = ruby_constant(head)
        const.is_a?(Module) ? ->(v) { v.is_a?(const) } : nil
      end
    end

    def ruby_constant(name)
      Object.const_get(name)
    rescue NameError
      nil
    end

    # `(T)` -> `T`, when the parentheses wrap the whole text.
    def strip_parens(t)
      while t.start_with?("(") && t.end_with?(")") && split_top(t[1..-2], ",").size == 1 && balanced?(t[1..-2])
        t = t[1..-2].strip
      end
      t
    end

    def balanced?(t)
      depth = 0
      t.each_char do |c|
        depth += 1 if "([".include?(c)
        depth -= 1 if ")]".include?(c)
        return false if depth.negative?
      end
      depth.zero?
    end

    # Split on `sep` at bracket depth zero.
    def split_top(t, sep)
      parts = []
      depth = 0
      cur = +""
      i = 0
      while i < t.length
        c = t[i]
        if "([".include?(c)
          depth += 1
        elsif ")]".include?(c)
          depth -= 1
        end
        if depth.zero? && t[i, sep.length] == sep && !(sep == "," && i.zero?)
          parts << cur.strip
          cur = +""
          i += sep.length
          next
        end
        cur << c
        i += 1
      end
      parts << cur.strip
      parts
    end
  end
end
