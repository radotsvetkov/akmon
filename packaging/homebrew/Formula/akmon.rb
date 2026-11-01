class Akmon < Formula
  desc "Tamper-evident evidence and verification layer for AI agents"
  homepage "https://github.com/radotsvetkov/akmon"
  version "3.0.0"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/radotsvetkov/akmon/releases/download/v3.0.0/akmon-darwin-arm64"
      sha256 "320f5c542bba637655fac8ef51f960d4dfa2280d51ae6ee8abf76588b05cef38"
    end
    on_intel do
      url "https://github.com/radotsvetkov/akmon/releases/download/v3.0.0/akmon-darwin-x86_64"
      sha256 "e16e7014ca743e745978d065e896e3f016855da4fdb0abf0ff1139cbd8b22ebb"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/radotsvetkov/akmon/releases/download/v3.0.0/akmon-linux-x86_64"
      sha256 "2a4c0c1c503447257d09e23e55631e1d15d25db6038e54b6b3b2556c47fe3af2"
    end
  end

  def install
    bin.install Dir["akmon-*"].first => "akmon"
  end

  test do
    assert_match "3.0.0", shell_output("#{bin}/akmon --version")
  end
end
