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

      # Declared field types, as written (`["String", "Int"]`), or nil.
      def field_types
        @field_types
      end
    end

    # Class-level information for Ruby classes made constructors of a type by
    # {HaskellMatch.sealed}; readers only, nothing of the class's behaviour
    # changes.
    module SealedClassMethods
      attr_reader :data_type, :constructor_name, :field_names, :arity

      def record?
        !field_names.nil?
      end

      def nullary?
        arity.zero?
      end

      # Sealed classes have no singleton value; `constructors` lists the class.
      def value
        nil
      end

      def field_types
        nil
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
    # fields, singleton values for nullary ones (classes for nullary sealed
    # classes, which have no singleton).
    def constructors
      constructor_classes.map { |k| k.value || k }
    end

    def constructor_names
      constructor_classes.map(&:constructor_name)
    end

    # `Maybe === Just.new(1)` -> true
    def ===(value)
      k = value.class
      k.respond_to?(:data_type) && k.data_type.equal?(self)
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
          elsif k.constructor_name.start_with?(":") && k.arity == 2
            "_ #{k.constructor_name} _"
          else
            [k.constructor_name, *Array.new(k.arity, "_")].join(" ")
          end
        end.join(" | ")
    end
  end

  class << self
    SYMBOL_WORDS = {
      ":" => "Colon", "+" => "Plus", "-" => "Minus", "*" => "Star", "/" => "Slash", "<" => "Lt", ">" => "Gt",
      "=" => "Eq", "!" => "Bang", "@" => "At", "#" => "Hash", "$" => "Dollar", "%" => "Percent", "&" => "Amp",
      "^" => "Caret", "|" => "Bar", "~" => "Tilde", "?" => "Query", "." => "Dot", "\\" => "Backslash"
    }.freeze

    # The Ruby constant for a constructor: its own name, or for an infix
    # constructor such as `:+:` a spelled-out one (`ColonPlusColon`).
    def constructor_constant(cname)
      return cname unless cname.start_with?(":")

      const = cname.chars.map { |c| SYMBOL_WORDS.fetch(c) { "U#{c.ord}" } }.join
      (@symbolic_constants ||= {})[const] = cname
      const
    end

    # The Haskell name of an infix constructor from its spelled-out constant
    # (`ColonPlusColon` -> `:+:`), or nil.
    def symbolic_constructor(const_name)
      (@symbolic_constants ||= {})[const_name.to_s]
    end

    # Copy the registrations of `type_names` from scope `src` into `dst`
    # (the Ruby side of `Native.import_scope`).
    def import_constructors(dst, src, type_names)
      @constructors ||= {}
      @type_modules ||= {}
      (@constructors[src] || {}).each do |n, c|
        k = c.is_a?(Class) ? c : c.class
        (@constructors[dst] ||= {})[n] = c if k.respond_to?(:data_type) && type_names.include?(k.data_type.type_name)
      end
      (@type_modules[src] || {}).each do |n, m|
        (@type_modules[dst] ||= {})[n] = m if type_names.include?(n)
      end
    end

    # Constructor classes and nullary values by name, as currently registered
    # (a redefined type replaces its constructors).
    def constructor(name, scope = Native::GLOBAL_SCOPE)
      @constructors ||= {}
      (@constructors[scope] || {})[name.to_s] || (scope != Native::GLOBAL_SCOPE ? constructor(name) : nil)
    end

    def register_constructors(classes, scope = Native::GLOBAL_SCOPE)
      @constructors ||= {}
      table = (@constructors[scope] ||= {})
      classes.each { |k| table[k.constructor_name] = k.value || k }
    end

    # The type module registered under `name` (in `scope`, falling back to
    # the global scope), or nil.
    def type_module(name, scope = Native::GLOBAL_SCOPE)
      @type_modules ||= {}
      (@type_modules[scope] || {})[name.to_s] || (scope != Native::GLOBAL_SCOPE ? type_module(name) : nil)
    end

    # Whether `HaskellMatch.data` checks field types by default (see
    # {FieldTypes}); off unless set.
    attr_writer :check_field_types

    def check_field_types
      @check_field_types ? true : false
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
    def data(decl, under: Object, scope: Native::GLOBAL_SCOPE, check_types: check_field_types, **constructors)
      name, tyvars, specs, deriving = normalize_data(decl, constructors)
      validate_type_name!(name)
      Deriving.validate!(name, specs, deriving)
      mod = new_type_module(name, tyvars, under)

      classes = specs.map do |cname, arity, fields, types|
        members = fields ? fields.map(&:to_sym) : (1..arity).map { |i| :"_#{i}" }
        klass = Data.define(*members)
        klass.include(Constructor)
        klass.instance_variable_set(:@data_type, mod)
        klass.instance_variable_set(:@constructor_name, cname)
        klass.instance_variable_set(:@field_names, fields&.map(&:to_sym))
        klass.instance_variable_set(:@arity, arity)
        klass.instance_variable_set(:@field_types, types&.map(&:to_s))
        const = constructor_constant(cname)
        mod.const_set(const, klass)
        klass.name # cache the permanent name
        FieldTypes.install(klass, types.map(&:to_s), scope) if check_types && types && types.size == arity && arity.positive?
        if arity.zero?
          value = klass.new.freeze
          klass.instance_variable_set(:@value, value)
          klass.singleton_class.send(:undef_method, :new)
          klass.singleton_class.send(:undef_method, :[])
          klass.singleton_class.send(:undef_method, :call)
          mod.send(:remove_const, const)
          mod.const_set(const, value)
        end
        klass
      end
      finish_type(mod, classes, under, scope)
      Deriving.apply(mod, classes, deriving)
      mod
    end

    # Make existing Ruby classes the constructors of a closed type, so their
    # instances match constructor patterns with full exhaustiveness checking
    # and no `HaskellMatch.data` declaration:
    #
    #   Circle = Data.define(:r)
    #   Rect   = Data.define(:w, :h)
    #   HaskellMatch.sealed :Shape, Circle, Rect
    #   area = HaskellMatch.fn(:area) { on(Circle(r)) { |r| 3 * r * r }; on("Rect w h") { |w, h| w * h } }
    #
    # `Data` and `Struct` classes supply their own field names (so record
    # patterns `Circle { r = x }` work too); for any other class pass the
    # reader methods that are its fields:
    #
    #   HaskellMatch.sealed :Shape, Circle => [:r], Rect => [:w, :h]
    #
    # The constructor name is the class's own name (`Geometry::Circle` is the
    # constructor `Circle`).  Instances are identified by exact class, so list
    # the leaf classes of a hierarchy.  Returns the type module, defined as a
    # constant under `under` when given (default: none).
    def sealed(name, *classes, under: nil, scope: Native::GLOBAL_SCOPE, **with_fields)
      name = name.to_s
      validate_type_name!(name)
      with_fields = with_fields.merge(classes.pop) if classes.last.is_a?(Hash)
      entries = classes.map { |k| [k, nil] } + with_fields.map { |k, f| [k, Array(f)] }
      raise DataDeclarationError, "type '#{name}' must have at least one constructor class" if entries.empty?

      mod = new_type_module(name, [], under)
      cons = entries.map do |klass, fields|
        raise DataDeclarationError, "#{klass.inspect} is not a Class" unless klass.is_a?(Class)

        cname = klass.name&.split("::")&.last
        if cname.nil? || !cname.match?(/\A[A-Z][A-Za-z0-9_']*\z/)
          raise DataDeclarationError, "#{klass.inspect} needs a constant name starting with an upper-case letter"
        end
        if klass.singleton_class.include?(Constructor::SealedClassMethods) || klass.include?(Constructor)
          raise DataDeclarationError, "#{klass} is already a constructor of type '#{klass.data_type.type_name}'"
        end

        fields ||= klass.members.map(&:to_s) if klass.respond_to?(:members)
        if fields.nil?
          raise DataDeclarationError,
                "#{klass} is neither a Data nor a Struct class: list its field readers (#{cname} => [:a, :b])"
        end
        fields = fields.map(&:to_s)
        klass.extend(Constructor::SealedClassMethods)
        klass.instance_variable_set(:@data_type, mod)
        klass.instance_variable_set(:@constructor_name, cname)
        klass.instance_variable_set(:@field_names, fields.map(&:to_sym))
        klass.instance_variable_set(:@arity, fields.size)
        mod.const_set(cname, klass)
        klass
      end
      begin
        finish_type(mod, cons, under, scope)
      rescue CompileError
        cons.each do |k|
          %i[@data_type @constructor_name @field_names @arity].each { |iv| k.remove_instance_variable(iv) }
          k.singleton_class.send(:undef_method, *Constructor::SealedClassMethods.instance_methods(false)) rescue nil # rubocop:disable Style/RescueModifier
        end
        raise
      end
      mod
    end

    private

    def new_type_module(name, tyvars, under)
      mod = Module.new
      mod.extend(DataType)
      mod.instance_variable_set(:@type_name, name)
      mod.instance_variable_set(:@type_variables, tyvars)
      mod.instance_variable_set(:@constructor_classes, [])
      # Name the module first so nullary constructor classes keep a permanent
      # class path even after their constant is replaced by the singleton.
      home = under || Types
      home.send(:remove_const, name) if home.const_defined?(name, false)
      home.const_set(name, mod)
      mod
    end

    # Register the type natively and in the Ruby registries; on failure the
    # constant is removed again.
    def finish_type(mod, classes, under, scope)
      mod.instance_variable_get(:@constructor_classes).concat(classes).freeze
      name = mod.type_name
      begin
        Native.register_type(name, classes.map { |k| [k.constructor_name, k.arity, k.field_names&.map(&:to_s), k] }, scope)
      rescue CompileError
        home = under || Types
        home.send(:remove_const, name) if home.const_defined?(name, false) && home.const_get(name, false).equal?(mod)
        raise
      end
      register_constructors(classes, scope)
      @type_modules ||= {}
      (@type_modules[scope] ||= {})[name] = mod
      mod
    end

    def normalize_data(decl, constructors)
      if decl.is_a?(String)
        unless constructors.empty?
          raise ArgumentError, "pass either a declaration string or constructor keywords, not both"
        end
        name, tyvars, cons, deriving = Native.parse_data(decl)
        [name, tyvars, cons, deriving]
      else
        name = decl.to_s
        deriving = Array(constructors.delete(:deriving)).map(&:to_s)
        if constructors.empty?
          raise DataDeclarationError, "type '#{name}' must have at least one constructor"
        end
        specs = constructors.map do |cname, spec|
          case spec
          when Integer
            raise DataDeclarationError, "arity of '#{cname}' must not be negative" if spec.negative?

            [cname.to_s, spec, nil, []]
          when Array
            [cname.to_s, spec.size, nil, spec.map(&:to_s)]
          when Hash
            fields = spec.keys.map(&:to_s)
            fields.each do |f|
              unless f.match?(/\A[a-z_][A-Za-z0-9_']*\z/)
                raise DataDeclarationError, "field name '#{f}' must start with a lower-case letter"
              end
            end
            [cname.to_s, fields.size, fields, spec.values.map(&:to_s)]
          when nil
            [cname.to_s, 0, nil, []]
          else
            raise DataDeclarationError,
                  "constructor '#{cname}' must be given an arity, an Array of field types or a Hash of fields"
          end
        end
        [name, [], specs, deriving]
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
