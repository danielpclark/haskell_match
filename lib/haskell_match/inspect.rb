# frozen_string_literal: true

module HaskellMatch
  # Haskell-style rendering of values, used by constructor `#inspect`.
  module Inspect
    module_function

    # Render `value` as a Haskell expression.  `atomic` requests parentheses
    # around anything that is an application (so it can appear as an
    # argument).
    def render(value, atomic: false)
      case value
      when Constructor
        render_constructor(value, atomic)
      when Array
        "[#{value.map { |v| render(v) }.join(', ')}]"
      when Integer, Float, Rational
        s = value.inspect
        value.negative? && atomic ? "(#{s})" : s
      when true then "True"
      when false then "False"
      else
        value.inspect
      end
    end

    def render_constructor(value, atomic)
      name = value.class.constructor_name
      fields = value.class.field_names
      values = value.fields
      return name if values.empty?

      if fields
        inner = fields.zip(values).map { |f, v| "#{f} = #{render(v)}" }.join(", ")
        "#{name} {#{inner}}"
      else
        s = "#{name} #{values.map { |v| render(v, atomic: true) }.join(' ')}"
        atomic ? "(#{s})" : s
      end
    end
  end
end
