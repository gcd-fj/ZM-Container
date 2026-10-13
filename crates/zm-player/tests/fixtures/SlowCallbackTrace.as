package {
    import flash.display.MovieClip;
    import flash.events.Event;
    import flash.utils.getTimer;

    public class SlowCallbackTrace extends MovieClip {
        private var privateValue:String = "DO_NOT_LOG_RECEIVER_72943";

        public function SlowCallbackTrace() {
            addEventListener(Event.ENTER_FRAME, onFrame);
        }

        private function onFrame(event:Event):void {
            removeEventListener(Event.ENTER_FRAME, onFrame);
            // A native sort calls back into script; both observations must be
            // captured without changing results or logging callback arguments.
            var entries:Array = [2, 1];
            entries.sort(compare);
            if (entries.join(",") != "1,2") throw new Error("sort changed");
            trace("SLOW_CALLBACK_TRACE_OK");
        }

        private function compare(left:int, right:int):Number {
            busyWork("DO_NOT_LOG_ARGUMENT_91872");
            return left - right;
        }

        private function busyWork(secret:String):void {
            var started:int = getTimer();
            var total:int = 0;
            // A clock-based duration makes the 100ms checkpoint observable on
            // fast and slow CI hosts; no specific elapsed value is asserted.
            while (getTimer() - started < 160) {
                for (var j:int = 0; j < 100; ++j) total += j;
            }
            if (secret.length == 0 || privateValue.length == 0) throw new Error("fixture");
        }
    }
}
