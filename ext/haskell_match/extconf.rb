# frozen_string_literal: true

# Builds the Rust extension when the gem is installed.  Instead of mkmf we
# emit a Makefile that delegates to cargo and copies the resulting library to
# the gem's lib/haskell_match/native directory.

require "rbconfig"
require "fileutils"

cargo = ENV["CARGO"] || "cargo"
unless system(cargo, "--version", out: File::NULL, err: File::NULL)
  abort "haskell_match requires a Rust toolchain (cargo) to build. See https://rustup.rs"
end

lib_name = case RbConfig::CONFIG["host_os"]
           when /darwin/ then "libhaskell_match.dylib"
           when /mswin|mingw|cygwin/ then "haskell_match.dll"
           else "libhaskell_match.so"
           end

ext_dir = __dir__
native_dir = File.expand_path("../../lib/haskell_match/native", __dir__)
FileUtils.mkdir_p(native_dir)

File.write("Makefile", <<~MAKEFILE)
  CARGO ?= #{cargo}
  EXT_DIR = #{ext_dir}
  NATIVE_DIR = #{native_dir}
  LIB = #{lib_name}

  all:
  \tcd $(EXT_DIR) && $(CARGO) build --release
  \tcp $(EXT_DIR)/target/release/$(LIB) $(NATIVE_DIR)/$(LIB)

  install: all

  clean:
  \tcd $(EXT_DIR) && $(CARGO) clean

  .PHONY: all install clean
MAKEFILE
