# frozen_string_literal: true

module HaskellMatch
  # Mixin for every constructor class produced by {HaskellMatch.data}.
  # Instances are frozen `Data` values; the class knows its type module,
  # constructor name and field names.
  module Constructor
    module ClassMethods
      attr_reader :data_type, :constructor_name, :field_names, :arity

      # `Just.(5)` -- Haskell constructors are functions.
      def call(*args, **kwargs)
        new(*args, **kwargs)
      end

      def to_proc
        method(:new).to_proc
      end

      def record?
        !field_names.nil?
      end

      def nullary?
        arity.zero?
      end

      # The nullary singleton value, or nil for constructors with fields.
      def value
        @value
      end
    end

    def self.included(base)
      base.extend(ClassMethods)
    end

    # Field values in declaration order.
    def fields
      to_h.values
    end

    def [](index)
      case index
      when Integer then fields[index]
      else public_send(index)
      end
    end

    def constructor_name
      self.class.constructor_name
    end

    def data_type
      self.class.data_type
    end

    def inspect
      Inspect.render(self)
    end
    alias to_s inspect
  end

  # The module returned by {HaskellMatch.data}; it holds a constant per
  # constructor so `include Maybe` brings `Just` and `Nothing` into scope.
  module DataType
    attr_reader :type_name, :type_variables, :constructor_classes

    # Constructors in declaration order: classes for constructors with
    # fields, singleton values for nullary ones.
    def constructors
      constructor_classes.map { |k| k.nullary? ? k.value : k }
    end

    def constructor_names
      constructor_classes.map(&:constructor_name)
    end

    # `Maybe === Just.new(1)` -> true
    def ===(value)
      value.is_a?(Constructor) && value.class.data_type.equal?(self)
    end

    # Haskell keeps types and constructors in separate namespaces; for
    # `data Person = Person {...}` the type module answers `new`, `[]` and
    # `call` with the same-named constructor, so `Person.new("Al", 3)` works
    # whether `Person` names the type or the constructor.
    def new(*args, **kwargs)
      same_named_constructor.new(*args, **kwargs)
    end

    def [](*args)
      same_named_constructor[*args]
    end

    def call(*args)
      same_named_constructor.call(*args)
    end

    def same_named_constructor
      k = constructor_classes.find { |c| c.constructor_name == type_name }
      raise NoMethodError, "#{inspect} has no constructor named #{type_name}" if k.nil?
      raise NoMethodError, "#{type_name} is a nullary constructor: use #{type_name}::#{type_name}" if k.nullary?

      k
    end
    private :same_named_constructor

    def inspect
      "data #{[type_name, *type_variables].join(' ')} = " +
        constructor_classes.map do |k|
          if k.record?
            "#{k.constructor_name} {#{k.field_names.join(', ')}}"
          else
            [k.constructor_name, *Array.new(k.arity, "_")].join(" ")
          end
        end.join(" | ")
    end
  end

  class << self
    # Constructor classes and nullary values by name, as currently registered
    # (a redefined type replaces its constructors).
    def constructor(name, scope = Native::GLOBAL_SCOPE)
      @constructors ||= {}
      (@constructors[scope] || {})[name.to_s] || (scope != Native::GLOBAL_SCOPE ? constructor(name) : nil)
    end

    def register_constructors(classes, scope = Native::GLOBAL_SCOPE)
      @constructors ||= {}
      table = (@constructors[scope] ||= {})
      classes.each { |k| table[k.constructor_name] = k.nullary? ? k.value : k }
    end

    # Declare an algebraic data type.
    #
    #   HaskellMatch.data "Maybe a = Nothing | Just a"
    #   HaskellMatch.data "Shape = Circle Double | Rect Double Double"
    #   HaskellMatch.data "Person = Person { name :: String, age :: Int }"
    #   HaskellMatch.data :Maybe, Nothing: 0, Just: 1          # arities
    #   HaskellMatch.data :Person, Person: { name: :String, age: :Int }
    #
    # Returns a module with one constant per constructor and defines it as a
    # constant named after the type under `under` (default: `Object`, i.e. a
    # top-level constant, like a Haskell `data` declaration).  Pass
    # `under: nil` to skip the constant definition.  `scope:` registers the
    # type in a type scope other than the global one (see {Native::GLOBAL_SCOPE}
    # and {Haskell#haskell_scope}).  Constructors with fields
    # are `Data` subclasses (`Just.new(1)`, `Just[1]`, `Just.(1)`); nullary
    # constructors are frozen singleton values (`Nothing`).
    def data(decl, under: Object, scope: Native::GLOBAL_SCOPE, **constructors)
      name, tyvars, specs = normalize_data(decl, constructors)
      validate_type_name!(name)

      mod = Module.new
      mod.extend(DataType)
      mod.instance_variable_set(:@type_name, name)
      mod.instance_variable_set(:@type_variables, tyvars)
      mod.instance_variable_set(:@constructor_classes, [])

      # Name the module first so nullary constructor classes keep a permanent
      # class path even after their constant is replaced by the singleton.
      home = under || Types
      if home.const_defined?(name, false)
        home.send(:remove_const, name)
      end
      home.const_set(name, mod)

      classes = specs.map do |cname, arity, fields|
        members = fields ? fields.map(&:to_sym) : (1..arity).map { |i| :"_#{i}" }
        klass = Data.define(*members)
        klass.include(Constructor)
        klass.instance_variable_set(:@data_type, mod)
        klass.instance_variable_set(:@constructor_name, cname)
        klass.instance_variable_set(:@field_names, fields&.map(&:to_sym))
        klass.instance_variable_set(:@arity, arity)
        mod.const_set(cname, klass)
        klass.name # cache the permanent name
        if arity.zero?
          value = klass.new.freeze
          klass.instance_variable_set(:@value, value)
          klass.singleton_class.send(:undef_method, :new)
          klass.singleton_class.send(:undef_method, :[])
          klass.singleton_class.send(:undef_method, :call)
          mod.send(:remove_const, cname)
          mod.const_set(cname, value)
        end
        klass
      end
      mod.instance_variable_get(:@constructor_classes).concat(classes).freeze

      begin
        Native.register_type(name, classes.map { |k| [k.constructor_name, k.arity, k.field_names&.map(&:to_s), k] }, scope)
      rescue CompileError
        home.send(:remove_const, name) if home.const_defined?(name, false)
        raise
      end
      register_constructors(classes, scope)
      mod
    end

    private

    def normalize_data(decl, constructors)
      if decl.is_a?(String)
        unless constructors.empty?
          raise ArgumentError, "pass either a declaration string or constructor keywords, not both"
        end
        name, tyvars, cons = Native.parse_data(decl)
        [name, tyvars, cons]
      else
        name = decl.to_s
        if constructors.empty?
          raise DataDeclarationError, "type '#{name}' must have at least one constructor"
        end
        specs = constructors.map do |cname, spec|
          case spec
          when Integer
            raise DataDeclarationError, "arity of '#{cname}' must not be negative" if spec.negative?

            [cname.to_s, spec, nil]
          when Array
            [cname.to_s, spec.size, nil]
          when Hash
            fields = spec.keys.map(&:to_s)
            fields.each do |f|
              unless f.match?(/\A[a-z_][A-Za-z0-9_']*\z/)
                raise DataDeclarationError, "field name '#{f}' must start with a lower-case letter"
              end
            end
            [cname.to_s, fields.size, fields]
          when nil
            [cname.to_s, 0, nil]
          else
            raise DataDeclarationError,
                  "constructor '#{cname}' must be given an arity, an Array of field types or a Hash of fields"
          end
        end
        [name, [], specs]
      end
    end

    def validate_type_name!(name)
      return if name.match?(/\A[A-Z][A-Za-z0-9_']*\z/)

      raise DataDeclarationError, "type name '#{name}' must start with an upper-case letter"
    end
  end

  # Home for types declared with `under: nil`.
  module Types; end
end
