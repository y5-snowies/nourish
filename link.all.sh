set -e
# The shared model: settings shapes, logging and stats, read by the compositor and
# by the external trees (installer, developer tool) alike. Nothing in it depends on
# a compositor crate — the generated block is what keeps that visible.
cd compositor.model
node ../workspace.link.js
cd ..

cd compositor.orchestration
node ../workspace.link.js
cd ..

cd compositor.expansion/compositor.y5
node ../../workspace.link.js
cd ..
cd ..
cd compositor.support
cd support.smithay
node ../../workspace.link.js

cd ..
cd ..
cd compositor.expansion/compositor.background
node ../../workspace.link.js
cd ..
cd ..

cd compositor.expansion/compositor.remote
node ../../workspace.link.js
cd ..
cd ..


# compositor.graphic dissolved into compositor.kernel/kernel.graphic — picked up
# by the kernel.* loop below.
for k in compositor.kernel/kernel.*; do
  (cd "$k" && node ../../workspace.link.js)
done





cd compositor.introspection
node ../workspace.link.js
cd ..

cd compositor.extension/compositor.monitor
node ../../workspace.link.js
cd ..
cd ..

cd compositor.extension/compositor.configurator
node ../../workspace.link.js
cd ..
cd ..

cd compositor.support
cd support.action
node ../../workspace.link.js
cd ..
cd support.system
node ../../workspace.link.js
cd ..
cd support.world
node ../../workspace.link.js
cd ..
cd support.library
node ../../workspace.link.js
cd ..
cd support.bevy
node ../../workspace.link.js
cd ..
cd support.iced
node ../../workspace.link.js
cd ..
cd ..


# Conformance gate: L0/L1/L2 layout + naming + size policy (document/ARCHITECTURE.md).
node workspace.lint.js
