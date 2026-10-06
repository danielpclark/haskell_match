# frozen_string_literal: true

require "rake/testtask"
require "rbconfig"
require "fileutils"

EXT_DIR = File.expand_path("ext/haskell_match", __dir__)
NATIVE_DIR = File.expand_path("lib/haskell_match/native", __dir__)

def native_library_name
  case RbConfig::CONFIG["host_os"]
  when /darwin/ then "libhaskell_match.dylib"
  when /mswin|mingw|cygwin/ then "haskell_match.dll"
  else "libhaskell_match.so"
  end
end

desc "Build the Rust extension with cargo (release) and copy it into lib/"
task :compile do
  profile = ENV["HASKELL_MATCH_PROFILE"] || "release"
  flags = profile == "release" ? ["--release"] : []
  sh "cargo", "build", *flags, chdir: EXT_DIR
  FileUtils.mkdir_p(NATIVE_DIR)
  src = File.join(EXT_DIR, "target", profile, native_library_name)
  FileUtils.cp(src, File.join(NATIVE_DIR, native_library_name))
  puts "copied #{src} -> #{NATIVE_DIR}"
end

namespace :cargo do
  desc "Run the Rust unit tests (pure core, no Ruby needed)"
  task :test do
    sh "cargo", "test", "--lib", chdir: EXT_DIR
  end

  desc "Run clippy"
  task :clippy do
    sh "cargo", "clippy", "--all-targets", chdir: EXT_DIR
  end

  desc "Check formatting"
  task :fmt do
    sh "cargo", "fmt", "--check", chdir: EXT_DIR
  end
end

Rake::TestTask.new(:test) do |t|
  t.libs << "test" << "lib"
  t.test_files = FileList["test/**/*_test.rb"]
  t.warning = true
end
task test: :compile

desc "Run the README introduction's examples and check their results"
task doctest: :compile do
  ruby "-Ilib", "test/support/doctest.rb", "README.md"
end

desc "Run the Ruby benchmarks"
task bench: :compile do
  ruby "-Ilib", "bench/bench.rb"
end

desc "Remove build products"
task :clean do
  FileUtils.rm_rf(File.join(EXT_DIR, "target"))
  FileUtils.rm_f(Dir[File.join(NATIVE_DIR, "*.{so,dylib,dll,bundle}")])
end

desc "Run Rust and Ruby tests"
task default: ["cargo:test", :test, :doctest]
