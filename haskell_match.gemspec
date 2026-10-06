# frozen_string_literal: true

require_relative "lib/haskell_match/version"

Gem::Specification.new do |spec|
  spec.name = "haskell_match"
  spec.version = HaskellMatch::VERSION
  spec.authors = ["Daniel P. Clark"]
  spec.email = ["6ftdan@gmail.com"]

  spec.summary = "Haskell-style pattern matching for Ruby, compiled to decision trees in Rust"
  spec.description = <<~DESC
    Algebraic data types and Haskell pattern syntax for Ruby.  Clauses are
    compiled once into a decision tree by a Rust extension (via Rutie) and
    checked for exhaustiveness and redundancy, so every logical path is
    accounted for before the first call.
  DESC
  spec.homepage = "https://github.com/danielpclark/haskell_match"
  spec.license = "MIT OR Apache-2.0"
  spec.required_ruby_version = ">= 3.2"

  spec.files = Dir[
    "lib/**/*.rb",
    "ext/haskell_match/Cargo.toml",
    "ext/haskell_match/Cargo.lock",
    "ext/haskell_match/extconf.rb",
    "ext/haskell_match/src/**/*.rs",
    "LICENSE-MIT", "LICENSE-APACHE", "README.md"
  ]
  spec.require_paths = ["lib"]
  spec.extensions = ["ext/haskell_match/extconf.rb"]

  spec.metadata["source_code_uri"] = spec.homepage
  spec.metadata["rubygems_mfa_required"] = "true"

  spec.add_development_dependency "minitest", "~> 5.0"
  spec.add_development_dependency "rake", "~> 13.0"
end
