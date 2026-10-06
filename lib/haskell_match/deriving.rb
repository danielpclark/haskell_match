# frozen_string_literal: true

module HaskellMatch
  # `deriving (...)` on a data declaration.  `Eq` and `Show` always hold for
  # constructor values (structural equality and Haskell-style `inspect`);
  # `Ord`, `Enum` and `Bounded` add the corresponding behaviour:
  #
  #   HaskellMatch.data "Color = Red | Green | Blue deriving (Eq, Ord, Enum, Bounded)"
  #   Red < Blue                   # => true  (Ord: by constructor order, then fields)
  #   Red.succ                     # => Green (Enum)
  #   (Red..Blue).to_a             # => [Red, Green, Blue]
  #   Color.enum_from(Green)       # => [Green, Blue]
  #   Color.min_bound              # => Red   (Bounded)
  module Deriving
    SUPPORTED = %w[Eq Show Ord Enum Bounded].freeze

    module_function

    # Reject unsupported classes and ill-typed derivations up front, before
    # the type is registered (`specs` are [name, arity, fields, types] rows).
    def validate!(type_name, specs, deriving)
      deriving.each do |cls|
        cls = cls.to_s
        unless SUPPORTED.include?(cls)
          raise DataDeclarationError,
                "cannot derive '#{cls}' for type '#{type_name}' (supported: #{SUPPORTED.join(', ')})"
        end
        next unless %w[Enum Bounded].include?(cls) && specs.any? { |(_, arity, *)| arity.positive? }

        raise DataDeclarationError,
              "cannot derive '#{cls}' for type '#{type_name}': it must be an enumeration type (all constructors nullary)"
      end
    end

    def apply(mod, classes, deriving)
      deriving.each do |cls|
        case cls.to_s
        when "Eq", "Show" then nil
        when "Ord" then ord(mod, classes)
        when "Enum" then enum(mod, classes)
        when "Bounded" then bounded(mod, classes)
        else
          raise DataDeclarationError,
                "cannot derive '#{cls}' for type '#{mod.type_name}' (supported: #{SUPPORTED.join(', ')})"
        end
      end
    end

    # Values compare by constructor order first, then field by field.
    def ord(mod, classes)
      tags = classes.each_with_index.to_h
      type = mod
      classes.each do |k|
        k.include(Comparable)
        k.define_method(:<=>) do |other|
          return nil unless other.is_a?(Constructor) && other.class.data_type.equal?(type)

          c = tags[self.class] <=> tags[other.class]
          return c unless c.zero?

          fields.zip(other.fields).each do |a, b|
            r = a <=> b
            return r if r.nil? || r != 0
          end
          0
        end
      end
    end

    # `succ`, `pred`, ranges and `Type.enum_from*`; only for enumerations
    # (every constructor nullary), as in GHC.
    def enum(mod, classes)
      unless classes.all?(&:nullary?)
        raise DataDeclarationError,
              "cannot derive 'Enum' for type '#{mod.type_name}': it must be an enumeration type (all constructors nullary)"
      end
      values = classes.map(&:value)
      type_name = mod.type_name
      values.each_with_index do |v, i|
        k = v.class
        k.define_method(:succ) do
          values[i + 1] or raise ArgumentError, "#{type_name}.succ: bad argument (#{self} is the last value)"
        end
        k.define_method(:pred) do
          i.positive? ? values[i - 1] : raise(ArgumentError, "#{type_name}.pred: bad argument (#{self} is the first value)")
        end
        k.define_method(:from_enum) { i }
        k.alias_method(:to_i, :from_enum)
        next if k.method_defined?(:<=>, false)

        k.include(Comparable)
        k.define_method(:<=>) do |other|
          other.is_a?(Constructor) && other.class.data_type.equal?(mod) ? i <=> other.from_enum : nil
        end
      end
      mod.define_singleton_method(:values) { values }
      mod.define_singleton_method(:to_enum) do |i|
        values[i] or raise ArgumentError, "#{type_name}.to_enum: bad argument #{i.inspect} (0..#{values.size - 1})"
      end
      mod.define_singleton_method(:from_enum) { |v| v.from_enum }
      mod.define_singleton_method(:enum_from) { |v| values[v.from_enum..] }
      mod.define_singleton_method(:enum_from_to) { |a, b| a > b ? [] : values[a.from_enum..b.from_enum] }
      mod.define_singleton_method(:enum_from_then) do |a, b|
        step = b.from_enum - a.from_enum
        raise ArgumentError, "#{type_name}.enum_from_then: the two values must differ" if step.zero?

        (a.from_enum..(step.positive? ? values.size - 1 : 0)).step(step.abs).map { |i| values[i] }
      end
      mod.define_singleton_method(:enum_from_then_to) do |a, b, c|
        step = b.from_enum - a.from_enum
        raise ArgumentError, "#{type_name}.enum_from_then_to: the first two values must differ" if step.zero?

        a.from_enum.step(c.from_enum, step).map { |i| values[i] }
      end
    end

    # `Type.min_bound` / `Type.max_bound`: the first and last constructor of
    # an enumeration.
    def bounded(mod, classes)
      unless classes.all?(&:nullary?)
        raise DataDeclarationError,
              "cannot derive 'Bounded' for type '#{mod.type_name}': only enumeration types are supported"
      end
      first = classes.first.value
      last = classes.last.value
      mod.define_singleton_method(:min_bound) { first }
      mod.define_singleton_method(:max_bound) { last }
    end
  end
end
