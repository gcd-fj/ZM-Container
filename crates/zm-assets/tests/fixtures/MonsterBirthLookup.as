// Original synthetic regression fixture, not redistributed game source.
package {
    import flash.display.MovieClip;
    import flash.utils.getTimer;
    import com.wdcgame.engine.managers.ManCreateManager;
    import com.wdcgame.engine.model.MonsterBirthVO;
    public class MonsterBirthLookup extends MovieClip {
        public function MonsterBirthLookup() {
            var manager:ManCreateManager = new ManCreateManager();
            if (manager.getMonsterBirthVo(1) !== null) throw new Error("empty");
            for (var i:int = -2; i < 40; ++i) manager.put(new MonsterBirthVO(i));
            for (i = -2; i < 40; ++i) {
                if (manager.getMonsterBirthVo(i) !== manager.expected(i)) throw new Error("identity");
            }
            if (manager.getMonsterBirthVo(100) !== null) throw new Error("missing");
            manager.remove(3);
            if (manager.getMonsterBirthVo(3) !== null) throw new Error("removed");
            manager.put(new MonsterBirthVO(3));
            if (manager.getMonsterBirthVo(3) !== manager.expected(3)) throw new Error("replaced");
            var shifted:MonsterBirthVO = manager.expected(4);
            shifted.id = 104;
            if (manager.getMonsterBirthVo(4) !== null || manager.getMonsterBirthVo(104) !== shifted) throw new Error("fallback");
            shifted.id = 4;
            manager.put(new MonsterBirthVO(int.MIN_VALUE));
            manager.put(new MonsterBirthVO(int.MAX_VALUE));
            if (manager.getMonsterBirthVo(int.MIN_VALUE) !== manager.expected(int.MIN_VALUE)) throw new Error("min");
            if (manager.getMonsterBirthVo(int.MAX_VALUE) !== manager.expected(int.MAX_VALUE)) throw new Error("max");
            var before:int = getTimer();
            var checksum:int = 0;
            for (var wave:int = 0; wave < 8; ++wave) {
                for each(var birth:MonsterBirthVO in manager.entries) {
                    for (var dependency:int = 0; dependency < 40; ++dependency) {
                        checksum += manager.getMonsterBirthVo(dependency).id;
                    }
                }
            }
            trace("LOOKUP_BENCH_MS=" + (getTimer() - before));
            if (checksum != 44 * 8 * 780) throw new Error("nested iteration checksum " + checksum);
            manager.clear();
            if (manager.getMonsterBirthVo(3) !== null) throw new Error("clear");
            manager.put(new MonsterBirthVO(3));
            if (manager.getMonsterBirthVo(3) !== manager.expected(3)) throw new Error("rebuild");
            trace("MONSTER_BIRTH_LOOKUP_OK");
        }
    }
}
package com.wdcgame.engine.model {
    public class MonsterBirthVO {
        public var id:int;
        public function MonsterBirthVO(id:int) { this.id = id; }
    }
}
package com.wdcgame.engine.managers {
    import flash.utils.Dictionary;
    import com.wdcgame.engine.model.MonsterBirthVO;
    public class ManCreateManager {
        private var _births:Dictionary = new Dictionary();
        public function get entries():Dictionary { return _births; }
        public function put(birth:MonsterBirthVO):void { buildBirthFromObjectVo(birth); }
        public function buildBirthFromObjectVo(birth:MonsterBirthVO):void { _births[birth.id] = birth; }
        public function expected(id:int):MonsterBirthVO { return _births[id]; }
        public function remove(id:int):void { delete _births[id]; }
        public function clear():void { _births = new Dictionary(); }
        public function getMonsterBirthVo(id:int):MonsterBirthVO {
            for each (var birth:MonsterBirthVO in this._births) {
                if (birth.id == id) return birth;
            }
            return null;
        }
    }
}
