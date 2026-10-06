# frozen_string_literal: true

require "fiddle"
require "rbconfig"

module HaskellMatch
  # Locates and loads the compiled Rust extension, then runs its `Init`
  # function.  The search order is:
  #
  # 1. `ENV["HASKELL_MATCH_NATIVE"]` (an explicit path to the library)
  # 2. `lib/haskell_match/native/` (where `rake compile` and `gem install` put it)
  # 3. `ext/haskell_match/target/{release,debug}/` (a fresh `cargo build`)
  module NativeLoader
    LIB_NAME = "haskell_match"

    def self.library_file_names
      base = "lib#{LIB_NAME}"
      exts = case RbConfig::CONFIG["host_os"]
             when /darwin/ then %w[dylib bundle so]
             when /mswin|mingw|cygwin/ then %w[dll so]
             else %w[so dylib]
             end
      exts.map { |e| "#{base}.#{e}" } + exts.map { |e| "#{LIB_NAME}.#{e}" }
    end

    def self.candidates
      root = File.expand_path("../..", __dir__)
      dirs = [
        File.join(root, "lib", "haskell_match", "native"),
        File.join(root, "ext", "haskell_match", "target", "release"),
        File.join(root, "ext", "haskell_match", "target", "debug")
      ]
      paths = dirs.product(library_file_names).map { |d, f| File.join(d, f) }
      explicit = ENV["HASKELL_MATCH_NATIVE"]
      paths.unshift(explicit) if explicit && !explicit.empty?
      paths
    end

    def self.find_library
      candidates.find { |p| File.file?(p) }
    end

    def self.load!
      return if @loaded

      path = find_library
      unless path
        raise LoadError,
              "haskell_match: compiled native library not found. Run `rake compile` " \
              "(or `cargo build --release` in ext/haskell_match). Looked in:\n  " +
              candidates.join("\n  ")
      end
      lib = Fiddle.dlopen(path)
      init = Fiddle::Function.new(lib["Init_#{LIB_NAME}"], [], Fiddle::TYPE_VOID)
      init.call
      @loaded = true
      @path = path
    end

    def self.path
      @path
    end
  end
end
