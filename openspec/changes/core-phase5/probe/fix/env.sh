# Scratch cores for the core-phase5 fix probes. Never the default or herdr session.
export FIX=/tmp/arena-p5/fix/h
export HC=$HOME/.local/bin/herdr-core
CLEAN="env -i HOME=$HOME USER=$USER PATH=/usr/bin:/bin:/usr/sbin:/sbin:$HOME/.local/bin TMPDIR=$TMPDIR LANG=en_US.UTF-8"
# local (near) core
L() { $CLEAN HC_SESSION=p5fix-loc XDG_STATE_HOME=$FIX/state XDG_RUNTIME_DIR=$FIX/run XDG_CONFIG_HOME=$FIX/config FIX=$FIX "$@"; }
# far core; F6 probe: started with fake herdr integration variables, as a far core a bare hc in a herdr pane would have
F() { $CLEAN HC_SESSION=p5fix-far XDG_STATE_HOME=$FIX/rstate XDG_RUNTIME_DIR=$FIX/rrun XDG_CONFIG_HOME=$FIX/rconfig FIX=$FIX "$@"; }
LSOCK=$FIX/run/herdr-core/p5fix-loc.sock
FSOCK=$FIX/rrun/herdr-core/p5fix-far.sock
