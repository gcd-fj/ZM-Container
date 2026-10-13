// Synthetic logger regression: no redistributed game source.
package {
    import flash.display.MovieClip;
    import flash.utils.getTimer;
    import flash.text.TextField;
    import com.wdcgame.utils.SimpleTextLog;
    public class HiddenTextLog extends MovieClip {
        public function HiddenTextLog() {
            var logger:SimpleTextLog = new SimpleTextLog();
            for (var i:int = 0; i < 1000; ++i) logger.log("missing spawn " + (i % 4));
            var started:int = getTimer();
            for (i = 0; i < 64; ++i) logger.log("missing spawn " + (i % 4));
            trace("HIDDEN_LOG_BENCH_MS=" + (getTimer() - started));
            var field:TextField = logger.txt;
            if (field.text.indexOf("missing spawn 0") < 0) throw new Error("hidden history lost");
            logger.reveal();
            if (!field.visible || field.text.indexOf("missing spawn 3") < 0) throw new Error("reveal");
            var before:String = field.text;
            logger.log("visible update");
            if (field.text != before + "-->visible update\n") throw new Error("visible append");
            before = field.text;
            logger.reveal();
            if (field.text != before) throw new Error("visible reveal changed text");
            field.visible = false;
            logger.log("hidden again");
            logger.reveal();
            if (field.text.indexOf("hidden again") < 0) throw new Error("second reveal");
            logger.logErr("error entry");
            if (logger.message.length != 1) throw new Error("error history");
            logger.failConsole = true;
            logger.log("exception preserved");
            if (logger.caught != 1) throw new Error("catch relocation");
            logger.failConsole = false;
            logger.clear();
            if (field.text != "") throw new Error("clear");
            logger.log("after clear");
            if (field.text != "-->after clear\n") throw new Error("clear then append");
            trace("HIDDEN_TEXT_LOG_OK");
        }
    }
}
package com.wdcgame.utils {
    import flash.text.TextField;
    import flash.text.TextFormat;
    import flash.events.MouseEvent;
    import flash.events.KeyboardEvent;
    public class SimpleTextLog {
        private var _txt:TextField = new TextField();
        private var __test:String = "";
        public var message:Array = [];
        public var failConsole:Boolean = false;
        public var caught:int = 0;
        public function SimpleTextLog() {
            _txt.width = 400; _txt.height = 160; _txt.wordWrap = true;
            _txt.defaultTextFormat = new TextFormat("_sans", 14);
            _txt.type = "input"; _txt.visible = false;
        }
        public function get txt():TextField { return _txt; }
        public function log(...rest):void {
            var line:String = "-->" + rest + "\n";
            __test += rest + "\n";
            _txt.appendText(line);
            _txt.scrollV = _txt.maxScrollV;
            trace("LOG=" + rest);
            try { console(); } catch (error:Error) { caught++; }
        }
        private function console():void { if (failConsole) throw new Error("console unavailable"); }
        public function logErr(...rest):void { message.push(rest); log(rest); }
        public function clear():void { _txt.text = ""; }
        public function reveal():void { onMouse(new MouseEvent("fixture", false, false, 1)); }
        protected function onMouse(event:MouseEvent):void {
            if (event.localX < 20) { _txt.visible = true; _txt.selectable = true; }
        }
        protected function __keyup(event:KeyboardEvent):void {
            _txt.visible = !_txt.visible;
            if (_txt.visible) _txt.text = __test;
        }
    }
}
